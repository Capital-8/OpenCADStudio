//! Plugin grip surface characterisation (API v11). Tests only; no
//! production code changed here.
//!
//! Paths covered (the commission's acceptance set):
//!   * read parity — HostApi `entity_grips` equals the automation `grips`
//!     read for the same single-selected entity (one pipeline, three
//!     consumers: viewport cache, MCP read, plugin read)
//!   * generality — arc (the 4-grip calibration shape), line, circle, a
//!     Spline (control-point grips) and a boxed-variant Helix
//!   * programmatic `grip_drag`: exactly one "GRIP"-labeled undo step with
//!     byte-exact restore; refusal parity (locked layer, reference-only,
//!     unknown grip/handle)
//!   * custom grips: declare/read parity, hover, press/hit (including
//!     coincident with a reference-only marker), drag phase flow, preview
//!     REPLACING per move with NO host ghost and the anchor entity not
//!     moving, settle writing from inside the callback in exactly one
//!     plugin-labeled step, Esc leaving the drawing byte-identical, axis
//!     hard-constraint, the absolute-point rule while the anchor moves,
//!     anchor-any-type (Spline/Helix), unload/death reaping, panic
//!     containment, budget caps
//!   * the coalescing model: 1000 parked points → one in flight, newest
//!     delivered next, settle never dropped (unit-level in
//!     `crate::plugin::grip_points`)

use std::cell::RefCell;
use std::rc::Rc;

use codec::entities::{Arc, Circle, Line, Spline};
use codec::types::{Handle, Vector3};
use codec::EntityType;
use ocs_plugin_api::host::{
    CustomGripEvent, CustomGripPhase, CustomGripResponse, PluginGrip, PluginGripShape, PreviewWire,
};

use crate::app::{Message, OpenCADStudio};
use crate::plugin::grip_points::{self, PluginGripHover};

fn app() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.automation_op(r#"{"op":"new"}"#);
    // A real canvas size (the projection round-trip the grip ops drive has
    // the viewport's resolution; the test default is a stub-sized pane).
    let i = tab(&app);
    app.tabs[i].scene.selection.borrow_mut().view.vp_size = (1600.0, 800.0);
    app
}

fn tab(app: &OpenCADStudio) -> usize {
    app.active_tab
}

fn add(app: &mut OpenCADStudio, entity: EntityType) -> Handle {
    let i = tab(app);
    let mut host = crate::app::plugin_host::HostSession::new(app, i);
    host.add_entity(entity)
}

fn line(start: [f64; 3], end: [f64; 3]) -> EntityType {
    EntityType::Line(Line::from_points(
        Vector3::new(start[0], start[1], start[2]),
        Vector3::new(end[0], end[1], end[2]),
    ))
}

fn arc(center: [f64; 3], radius: f64, start: f64, end: f64) -> EntityType {
    EntityType::Arc(Arc::from_coords(
        center[0], center[1], center[2], radius, start, end,
    ))
}

fn entity_grips(app: &mut OpenCADStudio, handle: Handle) -> Vec<ocs_plugin_api::host::GripInfo> {
    let i = tab(app);
    let host = crate::app::plugin_host::HostSession::new(app, i);
    host.entity_grips(handle).expect("grip read")
}

fn select(app: &mut OpenCADStudio, handle: Handle) {
    let i = tab(app);
    app.tabs[i].scene.deselect_all();
    app.tabs[i].scene.select_entity(handle, false);
    app.refresh_selected_grips();
}

fn control(app: &mut OpenCADStudio, mut op: serde_json::Value) -> serde_json::Value {
    // The protocol envelope the automation bridges supply: document_id,
    // revision, client_id and a unique request_id, with running states
    // drained (the block-ops test helper's shape).
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let state = app.control_state();
    op["protocol"] = serde_json::json!(1);
    op["document_id"] = state["document_id"].clone();
    op["revision"] = state["revision"].clone();
    op["client_id"] = serde_json::json!("test");
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    op["request_id"] = serde_json::json!(format!("grip-{id}"));
    let (response, task) = app.control_request(op.clone());
    let _ = app.drive_headless_task(task);
    if matches!(response["status"].as_str(), Some("accepted" | "running")) {
        app.control_request(serde_json::json!({
            "op": "operation",
            "request_id": op["request_id"],
        }))
        .0
    } else {
        response
    }
}

fn undo_entry_count(app: &OpenCADStudio) -> usize {
    let history = &app.tabs[tab(app)].history;
    history.undo_stack.len() + usize::from(history.pending.is_some())
}

fn last_undo_label(app: &OpenCADStudio) -> Option<String> {
    let history = &app.tabs[tab(app)].history;
    history.undo_stack.last().map(|s| s.label().to_string())
}

fn serialized(app: &OpenCADStudio, handle: Handle) -> Vec<u8> {
    let entity = app.tabs[tab(app)]
        .scene
        .document
        .get_entity(handle)
        .expect("entity exists")
        .clone();
    bincode::serialize(&entity).expect("entity serializes")
}

fn radius_of(app: &OpenCADStudio, handle: Handle) -> f64 {
    match app.tabs[tab(app)]
        .scene
        .document
        .get_entity(handle)
        .expect("entity exists")
    {
        EntityType::Arc(arc) => arc.radius,
        other => panic!("not an arc: {other:?}"),
    }
}

/// Screen point (tile-relative) of a world point through the active tile's
/// camera — the same projection the grips overlay uses.
fn screen_of(app: &OpenCADStudio, world: [f64; 3]) -> iced::Point {
    let i = tab(app);
    let (vw, vh) = app.tabs[i].scene.selection.borrow().view.vp_size;
    let tile = app.tabs[i].scene.active_model_tile_bounds(vw, vh);
    let bounds = iced::Rectangle {
        width: tile.width,
        height: tile.height,
        ..iced::Rectangle::default()
    };
    app.tabs[i]
        .scene
        .camera
        .borrow()
        .project(glam::DVec3::new(world[0], world[1], world[2]), bounds)
        .map(|screen| iced::Point::new(screen.x, screen.y))
        .expect("world point is in view")
}

fn declare(app: &mut OpenCADStudio, plugin: &str, grips: Vec<PluginGrip>) {
    let i = tab(app);
    let mut host = crate::app::plugin_host::HostSession::new(app, i);
    host.set_plugin_grips(plugin, grips)
        .expect("declaration accepted");
}

fn declared(grip_id: u64, point: [f64; 3]) -> PluginGrip {
    PluginGrip {
        grip_id,
        anchor: None,
        point,
        axis: None,
        shape: PluginGripShape::Square,
    }
}

/// A sink that records every event and can settle-write like the motivating
/// consumer (one push_undo scope + one entity update from inside the
/// callback).
struct SinkState {
    events: Rc<RefCell<Vec<CustomGripEvent>>>,
    /// (arc handle, new radius) the settle writes, when set.
    settle_write: Rc<RefCell<Option<(Handle, f64)>>>,
    /// Preview wire returned for begin/move.
    preview: Rc<RefCell<Option<PreviewWire>>>,
    /// Live-geometry entities returned for begin/move (API v12): the
    /// plugin-directed visual phase.
    live_geom: Rc<RefCell<Vec<(Handle, EntityType)>>>,
    /// Panic on the settle phase (panic-containment test).
    panic_on_settle: Rc<RefCell<bool>>,
}

fn install_sink(plugin: &str) -> SinkState {
    let state = SinkState {
        events: Rc::new(RefCell::new(Vec::new())),
        settle_write: Rc::new(RefCell::new(None)),
        preview: Rc::new(RefCell::new(None)),
        live_geom: Rc::new(RefCell::new(Vec::new())),
        panic_on_settle: Rc::new(RefCell::new(false)),
    };
    let events = Rc::clone(&state.events);
    let settle_write = Rc::clone(&state.settle_write);
    let preview = Rc::clone(&state.preview);
    let live_geom = Rc::clone(&state.live_geom);
    let panic_on_settle = Rc::clone(&state.panic_on_settle);
    grip_points::install_in_process_sink(plugin, move |host, event| {
        events.borrow_mut().push(event.clone());
        match event.phase {
            CustomGripPhase::Begin | CustomGripPhase::Move => CustomGripResponse {
                preview: preview.borrow().clone().into_iter().collect(),
                live_entities: live_geom.borrow().clone(),
            },
            CustomGripPhase::Settle => {
                if *panic_on_settle.borrow() {
                    panic!("plugin settle handler is buggy");
                }
                if let Some((handle, radius)) = *settle_write.borrow() {
                    // The consumer's exact shape: its own one-scope contract.
                    host.push_undo("CVI: PI radius");
                    if let Some(EntityType::Arc(arc)) = host.document().get_entity(handle).cloned()
                    {
                        let mut updated = arc;
                        updated.radius = radius;
                        assert!(host.update_entity(EntityType::Arc(updated)));
                    }
                }
                CustomGripResponse::default()
            }
            CustomGripPhase::Cancel => CustomGripResponse::default(),
        }
    });
    state
}

// ── Read parity + generality (requirement 1/6/9) ────────────────────────────

#[test]
fn entity_grips_arc_matches_the_four_grip_calibration() {
    let mut app = app();
    // R = 15 at a 90° corner: center + both tangent-point endpoints + the
    // radius grip at the parametric mid-angle of the arc.
    let handle = add(
        &mut app,
        arc([10.0, 10.0, 0.0], 15.0, 0.0, std::f64::consts::FRAC_PI_2),
    );
    let grips = entity_grips(&mut app, handle);
    assert_eq!(grips.len(), 4, "arc exposes exactly 4 grips: {grips:?}");
    assert_eq!(grips[0].grip_id, 0);
    assert!(grips[0].moves_entity, "grip 0 is the center translate grip");
    assert_eq!(grips[0].shape, "Square");
    assert_eq!(grips[0].point, [10.0, 10.0, 0.0]);
    let radius_grip = grips
        .iter()
        .find(|g| g.grip_id == 3)
        .expect("radius grip id 3");
    // Mid-angle of [0°, 90°] is 45°: the grip sits at center + R·(cos45, sin45).
    let mid = 15.0 * std::f64::consts::FRAC_1_SQRT_2;
    assert!((radius_grip.point[0] - (10.0 + mid)).abs() < 1e-9);
    assert!((radius_grip.point[1] - (10.0 + mid)).abs() < 1e-9);
    assert!(!radius_grip.moves_entity);
}

#[test]
fn entity_grips_line_and_circle_read() {
    let mut app = app();
    let line_handle = add(&mut app, line([0.0, 0.0, 0.0], [10.0, 0.0, 0.0]));
    let grips = entity_grips(&mut app, line_handle);
    assert!(grips.len() >= 3, "line: endpoints + midpoint: {grips:?}");
    assert!(grips.iter().any(|g| g.moves_entity));
    assert!(grips.iter().any(|g| g.point == [10.0, 0.0, 0.0]));

    let circle_handle = add(
        &mut app,
        EntityType::Circle(Circle::from_center_radius(Vector3::new(5.0, 5.0, 0.0), 4.0)),
    );
    let grips = entity_grips(&mut app, circle_handle);
    assert!(!grips.is_empty());
    assert!(
        grips
            .iter()
            .any(|g| g.moves_entity && g.point == [5.0, 5.0, 0.0]),
        "circle center translate grip: {grips:?}"
    );
}

#[test]
fn entity_grips_are_generic_over_entity_types_including_boxed_variants() {
    let mut app = app();
    // Spline: control-point grips.
    let spline = add(
        &mut app,
        EntityType::Spline(Spline::from_control_points(
            3,
            vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(10.0, 0.0, 0.0),
                Vector3::new(10.0, 10.0, 0.0),
                Vector3::new(0.0, 10.0, 0.0),
            ],
        )),
    );
    let grips = entity_grips(&mut app, spline);
    assert!(
        grips.len() >= 4,
        "spline exposes control-point grips: {grips:?}"
    );
    assert!(grips.iter().any(|g| g.point == [10.0, 0.0, 0.0]));

    // Helix: a recently-BOXED wide variant (`Box<Helix>`); the read must
    // match through the box.
    let helix = codec::entities::Helix {
        radius: 5.0,
        turns: 3.0,
        axis_base_point: Vector3::new(0.0, 0.0, 0.0),
        start_point: Vector3::new(5.0, 0.0, 0.0),
        ..codec::entities::Helix::default()
    };
    let helix_handle = add(&mut app, EntityType::Helix(Box::new(helix)));
    let grips = entity_grips(&mut app, helix_handle);
    assert!(
        !grips.is_empty(),
        "boxed helix grips must enumerate through the Box: {grips:?}"
    );
}

#[test]
fn host_api_read_matches_the_mcp_grips_read_for_the_same_entity() {
    // Requirement 9's drift test: the viewport cache, the automation read
    // and the plugin read all come from ONE pipeline, so a single-selected
    // entity must agree across all three.
    let mut app = app();
    let handle = add(
        &mut app,
        arc([0.0, 0.0, 0.0], 8.0, 0.0, std::f64::consts::PI),
    );
    select(&mut app, handle);
    // The viewport cache (what the interactive path hit-tests against):
    let cached = app.tabs[tab(&app)]
        .selected_grips
        .iter()
        .map(|g| {
            (
                g.id,
                g.world.to_array(),
                g.is_midpoint,
                format!("{:?}", g.shape),
            )
        })
        .collect::<Vec<_>>();
    // The MCP read:
    let read = control(&mut app, serde_json::json!({"op": "grips"}));
    assert_eq!(read["ok"], serde_json::json!(true));
    let mcp: Vec<(u64, [f64; 3], bool, String)> = read["grips"]
        .as_array()
        .expect("grips array")
        .iter()
        .map(|g| {
            (
                g["grip_id"].as_u64().expect("grip_id"),
                [
                    g["point"][0].as_f64().expect("x"),
                    g["point"][1].as_f64().expect("y"),
                    g["point"][2].as_f64().expect("z"),
                ],
                g["moves_entity"].as_bool().expect("moves_entity"),
                g["shape"].as_str().expect("shape").to_string(),
            )
        })
        .collect();
    // The HostApi read:
    let host_api = entity_grips(&mut app, handle)
        .into_iter()
        .map(|g| (g.grip_id, g.point, g.moves_entity, g.shape))
        .collect::<Vec<_>>();
    assert_eq!(cached.len(), mcp.len());
    assert_eq!(cached.len(), host_api.len());
    for (i, ((cid, cpoint, cmoves, cshape), (hid, hpoint, hmoves, hshape))) in
        cached.iter().zip(host_api.iter()).enumerate()
    {
        assert_eq!(cid, hid, "grip {i} id drifts");
        assert_eq!(cshape, hshape, "grip {i} shape drifts");
        assert_eq!(cmoves, hmoves, "grip {i} moves-entity drifts");
        for k in 0..3 {
            assert!(
                (cpoint[k] - hpoint[k]).abs() < 1e-12,
                "grip {i} point drifts: {cpoint:?} vs {hpoint:?}"
            );
        }
    }
    for (i, (mcp_entry, host_entry)) in mcp.iter().zip(host_api.iter()).enumerate() {
        assert_eq!(
            mcp_entry.0 as usize, host_entry.0,
            "MCP/HostApi grip {i} id drifts"
        );
        assert_eq!(mcp_entry.3, host_entry.3);
    }
}

// ── Programmatic grip drag (requirement 2) ──────────────────────────────────

#[test]
fn grip_drag_composes_one_undo_step_with_byte_exact_restore() {
    let mut app = app();
    let handle = add(
        &mut app,
        arc([0.0, 0.0, 0.0], 8.0, 0.0, std::f64::consts::PI),
    );
    let before = serialized(&app, handle);
    let before_undo = undo_entry_count(&app);

    let i = tab(&mut app);
    let mut host = crate::app::plugin_host::HostSession::new(&mut app, i);
    // Grip 0 is the center translate grip: the drag moves the whole entity
    // so the center lands exactly on `to` (the rubber-band anchor semantics
    // the interactive path uses).
    host.grip_drag(handle, 0, [12.0, 0.0, 0.0])
        .expect("grip drag applies");
    drop(host);

    // Exactly ONE new undo step, labeled like the interactive grip step.
    assert_eq!(undo_entry_count(&app), before_undo + 1);
    assert_eq!(last_undo_label(&app).as_deref(), Some("GRIP"));
    match app.tabs[tab(&app)]
        .scene
        .document
        .get_entity(handle)
        .expect("entity exists")
    {
        EntityType::Arc(arc) => {
            // The programmatic drag lands through the screen projection, so
            // the tolerance is the projection's own (far below one pixel).
            assert!((arc.center.x - 12.0).abs() < 1e-3);
            assert!(arc.center.y.abs() < 1e-3);
            assert!(
                (arc.radius - 8.0).abs() < 1e-9,
                "translate keeps the radius"
            );
        }
        other => panic!("not an arc: {other:?}"),
    }

    // Byte-exact restore on undo.
    let _ = app.update(Message::Undo);
    assert_eq!(undo_entry_count(&app), before_undo);
    assert_eq!(serialized(&app, handle), before);
}

#[test]
fn grip_drag_refusal_parity() {
    let mut app = app();
    let locked = add(&mut app, arc([0.0, 0.0, 0.0], 5.0, 0.0, 1.0));
    let reference = add(&mut app, arc([50.0, 0.0, 0.0], 5.0, 0.0, 1.0));

    // Reference-only first: the designation refuses on locked layers, and
    // both fixtures share layer "0" (only the first fixture gets locked).
    {
        let i = tab(&mut app);
        let mut host = crate::app::plugin_host::HostSession::new(&mut app, i);
        host.set_reference_only(reference, true).expect("mark");
        let grips = host.entity_grips(reference).expect("read");
        assert!(grips.is_empty(), "reference-only answers no grips");
        let ref_err = host.grip_drag(reference, 0, [1.0, 0.0, 0.0]).unwrap_err();
        assert!(
            ref_err.contains("reference_only") || ref_err.contains("unknown_grip"),
            "{ref_err}"
        );
    }

    // Lock the first entity's layer through the layers table.
    let i = tab(&app);
    let layer_name = app.tabs[i]
        .scene
        .document
        .get_entity(locked)
        .map(|entity| entity.common().layer.clone())
        .expect("entity layer");
    if let Some(layer) = app.tabs[i].scene.document.layers.get_mut(&layer_name) {
        layer.flags.locked = true;
    }

    // Locked-layer grip drags refuse with parity to the interactive path.
    let i = tab(&mut app);
    let mut host = crate::app::plugin_host::HostSession::new(&mut app, i);
    let locked_err = host.grip_drag(locked, 0, [1.0, 0.0, 0.0]).unwrap_err();
    assert!(
        locked_err.contains("layer_locked") || locked_err.contains("locked"),
        "{locked_err}"
    );
    drop(host);

    // Unlock, then the unknown-grip/unknown-handle refusals behave normally.
    if let Some(layer) = app.tabs[i].scene.document.layers.get_mut(&layer_name) {
        layer.flags.locked = false;
    }
    let plain = add(&mut app, arc([100.0, 0.0, 0.0], 5.0, 0.0, 1.0));
    let i = tab(&mut app);
    let mut host = crate::app::plugin_host::HostSession::new(&mut app, i);
    let unknown_grip = host.grip_drag(plain, 99, [1.0, 0.0, 0.0]).unwrap_err();
    assert!(unknown_grip.contains("unknown_grip"), "{unknown_grip}");
    let unknown_handle = host.grip_drag(Handle::from(0xFEED_F00Du64), 0, [1.0, 0.0, 0.0]);
    assert!(unknown_handle.is_err());
}

// ── Custom grips: declare, read, hover, hit, drag, settle, cancel ─────────

const TEST_PLUGIN: &str = "test.grip-points";

#[test]
fn custom_grips_declare_read_and_hover() {
    let mut app = app();
    let state = install_sink(TEST_PLUGIN);
    declare(
        &mut app,
        TEST_PLUGIN,
        vec![
            declared(7, [10.0, 10.0, 0.0]),
            declared(8, [30.0, 0.0, 0.0]),
        ],
    );
    // Read parity: the MCP read lists the plugin grips with their owner.
    let read = control(&mut app, serde_json::json!({"op": "grips"}));
    let plugin_grips = read["plugin_grips"].as_array().expect("plugin_grips");
    assert_eq!(plugin_grips.len(), 2);
    assert_eq!(plugin_grips[0]["plugin"], serde_json::json!(TEST_PLUGIN));
    assert_eq!(plugin_grips[0]["grip_id"], serde_json::json!(7));
    assert_eq!(
        plugin_grips[0]["point"],
        serde_json::json!([10.0, 10.0, 0.0])
    );
    assert_eq!(plugin_grips[0]["moves_entity"], serde_json::json!(false));

    // Whole-batch REPLACE: an empty batch clears.
    declare(&mut app, TEST_PLUGIN, Vec::new());
    let read = control(&mut app, serde_json::json!({"op": "grips"}));
    assert_eq!(
        read["plugin_grips"].as_array().expect("plugin_grips").len(),
        0
    );

    // Hover: the same overlay pipeline highlights the plugin grip.
    declare(&mut app, TEST_PLUGIN, vec![declared(7, [10.0, 10.0, 0.0])]);
    let i = tab(&mut app);
    let screen = screen_of(&app, [10.0, 10.0, 0.0]);
    app.update_grip_hover(i, screen);
    assert_eq!(
        app.plugin_grip_hover,
        Some(PluginGripHover {
            plugin: TEST_PLUGIN.to_string(),
            grip_id: 7,
        })
    );
    app.update_grip_hover(i, iced::Point::new(screen.x + 500.0, screen.y + 500.0));
    assert_eq!(app.plugin_grip_hover, None);

    grip_points::remove_in_process_sink(TEST_PLUGIN);
    let _ = state;
}

#[test]
fn custom_grip_drag_settles_as_one_plugin_labeled_step() {
    let mut app = app();
    let anchor = add(
        &mut app,
        arc([10.0, 10.0, 0.0], 8.0, 0.0, std::f64::consts::FRAC_PI_2),
    );
    let before = serialized(&app, anchor);
    let before_undo = undo_entry_count(&app);

    let state = install_sink(TEST_PLUGIN);
    // The calibration consumer: a custom grip at an implied PI — the tangent
    // intersection of the adjacent line directions — anchored to the arc,
    // with the corner's bisector as the hard drag axis.
    declare(
        &mut app,
        TEST_PLUGIN,
        vec![PluginGrip {
            grip_id: 42,
            anchor: Some(anchor),
            point: [10.0 + 8.0 * std::f64::consts::FRAC_1_SQRT_2, 10.0, 0.0],
            axis: None,
            shape: PluginGripShape::Square,
        }],
    );
    *state.settle_write.borrow_mut() = Some((anchor, 5.0));
    *state.preview.borrow_mut() = Some(PreviewWire::circle(
        [10.0, 10.0, 0.0],
        12.0,
        Some([1.0, 0.5, 0.0, 1.0]),
    ));

    let i = tab(&app);
    // Hover + press: the plugin grip engages through the same pipeline the
    // viewport uses.
    let screen = screen_of(
        &app,
        [10.0 + 8.0 * std::f64::consts::FRAC_1_SQRT_2, 10.0, 0.0],
    );
    let candidate = app
        .hit_plugin_grip(i, screen)
        .expect("the plugin grip is hittable");
    assert_eq!(candidate.grip_id, 42);
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    assert_eq!(state.events.borrow().len(), 1);
    assert_eq!(state.events.borrow()[0].phase, CustomGripPhase::Begin);
    assert_eq!(state.events.borrow()[0].grip_id, 42);
    // The begin answer's preview is already live.
    assert_eq!(app.tabs[i].scene.preview_wires.len(), 1);

    // Move: the plugin's preview REPLACES per event; the host draws no ghost
    // and moves nothing (the anchor arc is untouched mid-drag).
    let _ = app.move_plugin_grip_drag(i, glam::DVec3::new(18.0, 10.0, 0.0));
    let events = state.events.borrow();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].phase, CustomGripPhase::Move);
    assert_eq!(events[1].point, [18.0, 10.0, 0.0]);
    assert_eq!(
        app.tabs[i].scene.preview_wires.len(),
        1,
        "the plugin's preview is the only live feedback"
    );
    assert!(
        (radius_of(&app, anchor) - 8.0).abs() < 1e-12,
        "the anchor entity does not move by host action during the drag"
    );
    drop(events);

    // Settle: the plugin writes from INSIDE the callback — exactly ONE new
    // undo step, plugin-labeled; the preview clears.
    let _ = app.settle_plugin_grip_drag();
    let events = state.events.borrow();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].phase, CustomGripPhase::Settle);
    assert_eq!(events[2].point, [18.0, 10.0, 0.0]);
    drop(events);
    assert_eq!(undo_entry_count(&app), before_undo + 1);
    assert_eq!(
        last_undo_label(&app).as_deref(),
        Some("CVI: PI radius"),
        "the settled drag totals exactly the plugin's one labeled step"
    );
    assert!((radius_of(&app, anchor) - 5.0).abs() < 1e-12);
    assert!(app.tabs[i].scene.preview_wires.is_empty());

    // Undo restores the arc byte-exactly.
    let _ = app.update(Message::Undo);
    assert_eq!(undo_entry_count(&app), before_undo);
    assert_eq!(serialized(&app, anchor), before);

    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn custom_grip_escape_leaves_the_drawing_byte_identical() {
    let mut app = app();
    let anchor = add(&mut app, arc([0.0, 0.0, 0.0], 5.0, 0.0, 1.0));
    let before = serialized(&app, anchor);
    let before_undo = undo_entry_count(&app);
    let state = install_sink(TEST_PLUGIN);
    declare(&mut app, TEST_PLUGIN, vec![declared(1, [3.0, 3.0, 0.0])]);
    let i = tab(&app);
    let screen = screen_of(&app, [3.0, 3.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    let _ = app.move_plugin_grip_drag(i, glam::DVec3::new(9.0, 9.0, 0.0));
    // Esc cancels through the same lifecycle exit built-in drags use.
    assert!(app.cancel_active_grip_edit());
    let events = state.events.borrow();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].phase, CustomGripPhase::Cancel);
    drop(events);
    assert_eq!(undo_entry_count(&app), before_undo, "cancel writes nothing");
    assert_eq!(serialized(&app, anchor), before);
    assert!(app.tabs[i].scene.preview_wires.is_empty());
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn custom_grip_axis_hard_constrains_the_drag() {
    let mut app = app();
    let state = install_sink(TEST_PLUGIN);
    // Axis along +X through the grip point: a drag toward (20, 7) must
    // deliver points ON the constraint line (the hard-constraint decision).
    declare(
        &mut app,
        TEST_PLUGIN,
        vec![PluginGrip {
            grip_id: 3,
            anchor: None,
            point: [10.0, 0.0, 0.0],
            axis: Some([1.0, 0.0, 0.0]),
            shape: PluginGripShape::Circle,
        }],
    );
    let i = tab(&app);
    let screen = screen_of(&app, [10.0, 0.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    // Drive the cursor off-axis through the viewport's own move path: the
    // event carries the projected point on the constraint line.
    let off_axis = screen_of(&app, [20.0, 7.0, 0.0]);
    let (vw, vh) = app.tabs[i].scene.selection.borrow().view.vp_size;
    let tile = app.tabs[i].scene.active_model_tile_bounds(vw, vh);
    let _ = app.on_viewport_move(iced::Point::new(off_axis.x + tile.x, off_axis.y + tile.y));
    let events = state.events.borrow();
    let moved = events
        .iter()
        .rev()
        .find(|event| event.phase == CustomGripPhase::Move)
        .expect("a move was delivered");
    assert!(
        moved.point[1].abs() < 1e-6,
        "hard-constrained axis keeps the drag on the line, got {:?}",
        moved.point
    );
    assert!(moved.point[0] > 10.0, "the drag travels along the axis");
    drop(events);
    let _ = app.settle_plugin_grip_drag();
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn custom_grip_point_is_absolute_while_the_anchor_moves() {
    let mut app = app();
    let state = install_sink(TEST_PLUGIN);
    // Anchor accepts ANY live entity type — here a Spline — and NEVER adds
    // host-side movement semantics: the point stays exactly where declared.
    let spline = add(
        &mut app,
        EntityType::Spline(Spline::from_control_points(
            2,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(5.0, 5.0, 0.0)],
        )),
    );
    declare(
        &mut app,
        TEST_PLUGIN,
        vec![PluginGrip {
            grip_id: 9,
            anchor: Some(spline),
            point: [25.0, 40.0, 0.0],
            axis: None,
            shape: PluginGripShape::Triangle,
        }],
    );
    let i = tab(&app);
    // Events route to the owner through the anchor's tab.
    let screen = screen_of(&app, [25.0, 40.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");
    assert_eq!(candidate.anchor, Some(spline));
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    let _ = app.cancel_plugin_grip_drag();

    // The host re-declares nothing on its own: the plugin's pull model. The
    // anchor entity's move delivers no events at all.
    state.events.borrow_mut().clear();
    // Move the anchor entity: the declared point does not follow.
    let moved = match app.tabs[i].scene.document.get_entity(spline).cloned() {
        Some(EntityType::Spline(spline)) => spline,
        other => panic!("not a spline: {other:?}"),
    };
    let mut moved = moved;
    moved.control_points[0] = Vector3::new(500.0, 500.0, 0.0);
    let mut host = crate::app::plugin_host::HostSession::new(&mut app, i);
    assert!(host.update_entity(EntityType::Spline(moved)));
    drop(host);
    let read = control(&mut app, serde_json::json!({"op": "grips"}));
    let plugin_grips = read["plugin_grips"].as_array().expect("plugin_grips");
    assert_eq!(plugin_grips.len(), 1);
    assert_eq!(
        plugin_grips[0]["point"],
        serde_json::json!([25.0, 40.0, 0.0]),
        "the host never moves, offsets or auto-follows a custom grip's point"
    );
    // The host re-declares nothing on its own: the plugin's pull model.
    assert!(state.events.borrow().is_empty());
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn custom_grip_wins_the_hit_over_a_coincident_reference_only_marker() {
    let mut app = app();
    let marker = add(
        &mut app,
        EntityType::Circle(Circle::from_center_radius(
            Vector3::new(10.0, 10.0, 0.0),
            4.0,
        )),
    );
    {
        let i = tab(&app);
        let mut host = crate::app::plugin_host::HostSession::new(&mut app, i);
        host.set_reference_only(marker, true).expect("mark");
    }
    let _state = install_sink(TEST_PLUGIN);
    // The grip sits EXACTLY ON the plugin's own reference-only outline.
    declare(&mut app, TEST_PLUGIN, vec![declared(5, [10.0, 10.0, 0.0])]);
    let i = tab(&app);
    let screen = screen_of(&app, [10.0, 10.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen);
    assert!(
        candidate.is_some(),
        "a custom grip coincident with its own reference-only marker stays fully hittable"
    );
    // And the press engages the drag (not the click-through entity pick).
    let candidate = candidate.unwrap();
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    assert!(app.tabs[i].active_plugin_grip.is_some());
    let _ = app.cancel_plugin_grip_drag();
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn custom_grips_die_with_their_plugin_and_nothing_else_changes() {
    let mut app = app();
    let other_entity = add(&mut app, line([0.0, 0.0, 0.0], [5.0, 0.0, 0.0]));
    let _state_a = install_sink("grip.plugin-a");
    let _state_b = install_sink("grip.plugin-b");
    declare(
        &mut app,
        "grip.plugin-a",
        vec![declared(1, [1.0, 1.0, 0.0])],
    );
    declare(
        &mut app,
        "grip.plugin-b",
        vec![declared(2, [2.0, 2.0, 0.0])],
    );
    let other_before = serialized(&app, other_entity);
    // Plugin A dies (its sink is removed): the reap clears exactly its set.
    grip_points::remove_in_process_sink("grip.plugin-a");
    app.reap_plugin_grips();
    let read = control(&mut app, serde_json::json!({"op": "grips"}));
    let plugin_grips = read["plugin_grips"].as_array().expect("plugin_grips");
    assert_eq!(plugin_grips.len(), 1);
    assert_eq!(
        plugin_grips[0]["plugin"],
        serde_json::json!("grip.plugin-b")
    );
    assert_eq!(serialized(&app, other_entity), other_before);
    grip_points::remove_in_process_sink("grip.plugin-b");
}

#[test]
fn disabled_plugins_grips_are_invisible() {
    let mut app = app();
    let _state = install_sink(TEST_PLUGIN);
    declare(&mut app, TEST_PLUGIN, vec![declared(1, [1.0, 1.0, 0.0])]);
    let i = tab(&app);
    let screen = screen_of(&app, [1.0, 1.0, 0.0]);
    assert!(app.hit_plugin_grip(i, screen).is_some());
    app.disabled_plugins.insert(TEST_PLUGIN.to_string());
    assert!(
        app.hit_plugin_grip(i, screen).is_none(),
        "disabled plugins' grips disappear"
    );
    let read = control(&mut app, serde_json::json!({"op": "grips"}));
    assert_eq!(
        read["plugin_grips"].as_array().expect("plugin_grips").len(),
        0
    );
    app.disabled_plugins.remove(TEST_PLUGIN);
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn a_panicking_grip_handler_never_takes_the_host_down() {
    let mut app = app();
    let anchor = add(&mut app, arc([0.0, 0.0, 0.0], 5.0, 0.0, 1.0));
    let before = serialized(&app, anchor);
    let before_undo = undo_entry_count(&app);
    let state = install_sink(TEST_PLUGIN);
    declare(&mut app, TEST_PLUGIN, vec![declared(1, [2.0, 2.0, 0.0])]);
    *state.panic_on_settle.borrow_mut() = true;
    *state.settle_write.borrow_mut() = Some((anchor, 1.0));
    let i = tab(&app);
    let screen = screen_of(&app, [2.0, 2.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    let _ = app.move_plugin_grip_drag(i, glam::DVec3::new(3.0, 3.0, 0.0));
    // The settle handler panics: the guard contains it, the host survives,
    // the drawing is untouched and no undo step is recorded.
    let _ = app.settle_plugin_grip_drag();
    assert_eq!(serialized(&app, anchor), before);
    assert_eq!(undo_entry_count(&app), before_undo);
    assert!(
        !crate::plugin::drain_errors().is_empty(),
        "the panic is reported"
    );
    // The host keeps working afterwards.
    let handle = add(&mut app, line([0.0, 0.0, 0.0], [1.0, 1.0, 0.0]));
    assert!(handle.value() > 0);
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn mcp_grip_drag_routes_a_custom_grip_to_its_owner() {
    // MCP parity: declare → read → drag through the automation op, exactly
    // the recipe the consumer tests with.
    let mut app = app();
    let anchor = add(&mut app, arc([0.0, 0.0, 0.0], 8.0, 0.0, 1.0));
    let before_undo = undo_entry_count(&app);
    let state = install_sink(TEST_PLUGIN);
    declare(&mut app, TEST_PLUGIN, vec![declared(11, [8.0, 0.0, 0.0])]);
    *state.settle_write.borrow_mut() = Some((anchor, 4.0));
    let outcome = control(
        &mut app,
        serde_json::json!({
            "op": "grip_drag",
            "plugin": TEST_PLUGIN,
            "grip_id": 11,
            "point": [9.0, 0.0, 0.0],
        }),
    );
    assert_eq!(outcome["ok"], serde_json::json!(true), "{outcome}");
    let events = state.events.borrow();
    let phases: Vec<CustomGripPhase> = events.iter().map(|e| e.phase).collect();
    assert_eq!(
        phases,
        vec![
            CustomGripPhase::Begin,
            CustomGripPhase::Move,
            CustomGripPhase::Settle
        ]
    );
    drop(events);
    assert_eq!(undo_entry_count(&app), before_undo + 1);
    assert_eq!(last_undo_label(&app).as_deref(), Some("CVI: PI radius"));
    assert!((radius_of(&app, anchor) - 4.0).abs() < 1e-12);

    // Unknown plugin-grip id refuses.
    let refused = control(
        &mut app,
        serde_json::json!({
            "op": "grip_drag",
            "plugin": TEST_PLUGIN,
            "grip_id": 999,
            "point": [9.0, 0.0, 0.0],
        }),
    );
    assert_eq!(refused["ok"], serde_json::json!(false), "{refused}");
    assert!(
        refused.to_string().contains("unknown_grip"),
        "the refusal names the code: {refused}"
    );
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn hover_hits_through_two_thousand_declared_grips() {
    // Requirement 8's scale case: a street network's worth of declared
    // grips; hover goes through the per-tab uniform grid with view culling,
    // never a per-grip linear scan per cursor move.
    let mut app = app();
    let _state = install_sink(TEST_PLUGIN);
    let grips: Vec<PluginGrip> = (0..2000)
        .map(|i| {
            declared(
                i as u64,
                [(i % 100) as f64 * 0.003, (i / 100) as f64 * 0.003, 0.0],
            )
        })
        .collect();
    declare(&mut app, TEST_PLUGIN, grips);
    let i = tab(&app);
    // Grip 53 sits at (0.159, 0); hover right on it.
    let screen = screen_of(&app, [0.159, 0.0, 0.0]);
    let hit = app
        .hit_plugin_grip(i, screen)
        .expect("found through the grid");
    assert_eq!(hit.grip_id, 53);
    // A point far outside the cluster hovers nothing.
    let bare = screen_of(&app, [5.0, 5.0, 0.0]);
    assert!(app.hit_plugin_grip(i, bare).is_none());
    // Interactive-rate bound: 1 000 hovers over the 2 000-grip scene.
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let got = app.hit_plugin_grip(i, screen);
        debug_assert!(got.is_some());
    }
    let per_hover = start.elapsed() / 1000;
    assert!(
        per_hover < std::time::Duration::from_millis(20),
        "hover stays interactive at 2 000 grips: {per_hover:?}"
    );
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

fn live_arc(anchor: Handle, radius: f64, app: &OpenCADStudio) -> codec::EntityType {
    match app.tabs[tab(app)]
        .scene
        .document
        .get_entity(anchor)
        .expect("anchor exists")
    {
        EntityType::Arc(arc) => {
            let mut live = arc.clone();
            live.radius = radius;
            EntityType::Arc(live)
        }
        other => panic!("not an arc: {other:?}"),
    }
}

#[test]
fn live_geometry_moves_elements_during_the_drag() {
    // v11.2 acceptance: the plugin's move answers carry live entity
    // geometry — the drag shows the true final state at every instant
    // through the plugin's returned geometry only (visual parity with the
    // built-in radius grip), ZERO history steps mid-drag, the originals
    // restored before the settle, and the plugin's one labeled step at
    // mouse-up with byte-exact undo.
    let mut app = app();
    let anchor = add(
        &mut app,
        arc([10.0, 10.0, 0.0], 8.0, 0.0, std::f64::consts::FRAC_PI_2),
    );
    let before = serialized(&app, anchor);
    let before_undo = undo_entry_count(&app);
    let state = install_sink(TEST_PLUGIN);
    declare(
        &mut app,
        TEST_PLUGIN,
        vec![PluginGrip {
            grip_id: 42,
            anchor: Some(anchor),
            point: [12.0, 10.0, 0.0],
            axis: None,
            shape: PluginGripShape::Square,
        }],
    );
    *state.settle_write.borrow_mut() = Some((anchor, 5.0));
    *state.preview.borrow_mut() = Some(PreviewWire::circle(
        [10.0, 10.0, 0.0],
        12.0,
        Some([1.0, 0.5, 0.0, 1.0]),
    ));
    // The plugin's live answer: the arc's candidate geometry for the move.
    *state.live_geom.borrow_mut() = vec![(anchor, live_arc(anchor, 6.25, &app))];

    let i = tab(&app);
    let screen = screen_of(&app, [12.0, 10.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    let _ = app.pump_plugin_grips();
    assert!(
        (radius_of(&app, anchor) - 6.25).abs() < 1e-9,
        "the begin answer's live geometry already moves the element"
    );
    assert_eq!(undo_entry_count(&app), before_undo, "zero steps mid-drag");
    let _ = app.move_plugin_grip_drag(i, glam::DVec3::new(14.0, 10.0, 0.0));
    assert!(
        (radius_of(&app, anchor) - 6.25).abs() < 1e-9,
        "the move answer's live geometry is applied live"
    );
    assert_eq!(app.tabs[i].scene.preview_wires.len(), 1);
    assert_eq!(
        undo_entry_count(&app),
        before_undo,
        "still zero steps mid-drag"
    );

    // Mouse-up: the visual phase is restored BEFORE the settle, the plugin
    // writes its authoritative edit inside its scope — exactly ONE step.
    let _ = app.settle_plugin_grip_drag();
    assert!((radius_of(&app, anchor) - 5.0).abs() < 1e-12);
    assert_eq!(undo_entry_count(&app), before_undo + 1);
    assert_eq!(last_undo_label(&app).as_deref(), Some("CVI: PI radius"));
    let _ = app.update(Message::Undo);
    assert_eq!(serialized(&app, anchor), before, "undo restores byte-exact");
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn live_geometry_esc_discards_the_visual_state_byte_identically() {
    let mut app = app();
    let anchor = add(&mut app, arc([0.0, 0.0, 0.0], 5.0, 0.0, 1.0));
    let before = serialized(&app, anchor);
    let before_undo = undo_entry_count(&app);
    let state = install_sink(TEST_PLUGIN);
    declare(&mut app, TEST_PLUGIN, vec![declared(3, [3.0, 0.0, 0.0])]);
    *state.live_geom.borrow_mut() = vec![(anchor, live_arc(anchor, 9.0, &app))];
    let i = tab(&app);
    let screen = screen_of(&app, [3.0, 0.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    let _ = app.pump_plugin_grips();
    assert!((radius_of(&app, anchor) - 9.0).abs() < 1e-12, "live moved");
    assert!(app.cancel_active_grip_edit());
    assert_eq!(
        serialized(&app, anchor),
        before,
        "Esc restores the visual-only state byte-identically"
    );
    assert_eq!(undo_entry_count(&app), before_undo, "cancel writes nothing");
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn live_geometry_caps_are_refused_with_an_honest_headline() {
    let mut app = app();
    let anchor = add(&mut app, arc([0.0, 0.0, 0.0], 5.0, 0.0, 1.0));
    let state = install_sink(TEST_PLUGIN);
    declare(&mut app, TEST_PLUGIN, vec![declared(1, [3.0, 0.0, 0.0])]);
    let overflow = ocs_plugin_api::host::MAX_PLUGIN_LIVE_ENTITIES + 1;
    let way_too_big: Vec<(codec::Handle, codec::EntityType)> = (0..overflow)
        .map(|k| (anchor, live_arc(anchor, 5.0 + k as f64, &app)))
        .collect();
    *state.live_geom.borrow_mut() = way_too_big;
    let i = tab(&app);
    let screen = screen_of(&app, [3.0, 0.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    let _ = app.pump_plugin_grips();
    assert!(
        (radius_of(&app, anchor) - 5.0).abs() < 1e-12,
        "a too-large live payload applies nothing"
    );
    let errors = crate::plugin::drain_errors();
    assert!(
        errors
            .iter()
            .any(|line| line.contains("exceeds the per-response cap")),
        "the refusal names the cap: {errors:?}"
    );
    let _ = app.cancel_plugin_grip_drag();
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn entities_batch_reads_live_handles_without_snapshot_clones() {
    // v11.1 defect 3: per-entity reads are the refresh-pass path — the
    // answers mirror the live document exactly, unknown handles answer
    // None, and no snapshot is involved.
    let mut app = app();
    let first = add(&mut app, arc([0.0, 0.0, 0.0], 5.0, 0.0, 1.0));
    let second = add(&mut app, line([0.0, 0.0, 0.0], [9.0, 0.0, 0.0]));
    let i = tab(&app);
    let batch = {
        let host = crate::app::plugin_host::HostSession::new(&mut app, i);
        host.entities_batch(&[first, second, Handle::from(0xFEED_F00Du64)])
            .expect("batch read")
    };
    assert_eq!(batch.len(), 3);
    let first_read = batch[0]
        .as_ref()
        .map(|entity| bincode::serialize(entity).unwrap());
    assert_eq!(
        first_read,
        Some(serialized(&app, first)),
        "the first handle reads byte-equal"
    );
    assert!(
        matches!(&batch[1], Some(EntityType::Line(_))),
        "the second handle reads by kind"
    );
    assert!(batch[2].is_none(), "an unknown handle answers None");
}

#[test]
fn calibration_measurements_for_the_report() {
    // Requirement 8's reported measurements, as bounds a CI can hold:
    // the move-phase delivery rate, the settle-write latency (with the
    // plugin writing from inside the callback) and the unchanged
    // re-declare diff at 1 000 grips (the pull model's boundary cost).
    // Run with --nocapture to print the measured values.
    let mut app = app();
    let anchor = add(
        &mut app,
        arc([0.0, 0.0, 0.0], 8.0, 0.0, std::f64::consts::FRAC_PI_2),
    );
    let state = install_sink(TEST_PLUGIN);
    declare(
        &mut app,
        TEST_PLUGIN,
        vec![PluginGrip {
            grip_id: 1,
            anchor: Some(anchor),
            point: [4.0, 4.0, 0.0],
            axis: None,
            shape: PluginGripShape::Square,
        }],
    );
    *state.settle_write.borrow_mut() = Some((anchor, 5.0));
    *state.preview.borrow_mut() = Some(PreviewWire::circle(
        [0.0, 0.0, 0.0],
        6.0,
        Some([1.0, 0.6, 0.0, 1.0]),
    ));
    let i = tab(&app);
    let screen = screen_of(&app, [4.0, 4.0, 0.0]);
    let candidate = app.hit_plugin_grip(i, screen).expect("hittable");

    // Move-phase: 1 000 cursor points through the channel (in-process
    // delivery is synchronous, so this bounds the app-side machinery).
    let _ = app.begin_plugin_grip_drag(i, &candidate);
    let start = std::time::Instant::now();
    for k in 0..1000u32 {
        let _ = app.move_plugin_grip_drag(i, glam::DVec3::new(4.0, 4.0, k as f64 * 0.01));
    }
    let moves = start.elapsed();
    let per_move = moves / 1000;

    // Settle-write: the full begin → move → settle-with-writes window.
    let start = std::time::Instant::now();
    let _ = app.settle_plugin_grip_drag();
    let settle = start.elapsed();
    assert_eq!(last_undo_label(&app).as_deref(), Some("CVI: PI radius"));
    assert!((radius_of(&app, anchor) - 5.0).abs() < 1e-12);

    // Re-declare diff at 1 000 grips: the unchanged batch must stay cheap.
    let batch: Vec<PluginGrip> = (0..1000)
        .map(|k| declared(k as u64, [k as f64, 42.0, 0.0]))
        .collect();
    declare(&mut app, TEST_PLUGIN, batch);
    let batch: Vec<PluginGrip> = (0..1000)
        .map(|k| declared(k as u64, [k as f64, 42.0, 0.0]))
        .collect();
    let start = std::time::Instant::now();
    declare(&mut app, TEST_PLUGIN, batch);
    let redeclare = start.elapsed();

    eprintln!("grip-surface calibration: 1000 move events = {moves:?} ({per_move:?}/move), settle-with-write = {settle:?}, unchanged 1000-grip re-declare = {redeclare:?}");
    assert!(per_move < std::time::Duration::from_millis(5));
    assert!(settle < std::time::Duration::from_millis(500));
    assert!(redeclare < std::time::Duration::from_millis(25));
    grip_points::remove_in_process_sink(TEST_PLUGIN);
}

#[test]
fn declaration_validates_anchors_and_caps() {
    let mut app = app();
    let _state = install_sink(TEST_PLUGIN);
    let i = tab(&app);
    let mut host = crate::app::plugin_host::HostSession::new(&mut app, i);
    // An anchor that does not exist is refused (any live type is fine).
    let err = host
        .set_plugin_grips(
            TEST_PLUGIN,
            vec![PluginGrip {
                grip_id: 1,
                anchor: Some(Handle::from(0xDEAD_BEEFu64)),
                point: [0.0; 3],
                axis: None,
                shape: PluginGripShape::Square,
            }],
        )
        .unwrap_err();
    assert!(err.contains("does not exist"), "{err}");
    // An unknown plugin cannot declare at all.
    let err = host
        .set_plugin_grips("not.a.plugin", vec![declared(1, [0.0; 3])])
        .unwrap_err();
    assert!(err.contains("not a loaded, alive plugin"), "{err}");
    // Per-plugin cap.
    let huge: Vec<PluginGrip> = (0..=grip_points::MAX_PLUGIN_GRIPS_PER_PLUGIN)
        .map(|i| declared(i as u64, [i as f64, 0.0, 0.0]))
        .collect();
    assert!(host.set_plugin_grips(TEST_PLUGIN, huge).is_err());
}
