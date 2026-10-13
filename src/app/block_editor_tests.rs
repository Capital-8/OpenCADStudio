use super::{Message, ModalKind, OpenCADStudio};
use codec::entities::{Insert, Line};
use codec::types::Vector3;
use codec::{EntityType, Handle};

fn fixture() -> (OpenCADStudio, Handle) {
    let mut app = OpenCADStudio::new_for_test();
    app.automation_op(r#"{"op":"new"}"#);
    let i = app.active_tab;
    app.tabs[i].scene.define_block_raw(
        "EDIT_ME",
        Vector3::ZERO,
        vec![EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::UNIT_X,
        ))],
    );
    let reference = app.tabs[i]
        .scene
        .document
        .add_entity(EntityType::Insert(Insert::new("EDIT_ME", Vector3::ZERO)))
        .unwrap();
    app.tabs[i].dirty = false;
    (app, reference)
}

fn begin(app: &mut OpenCADStudio, reference: Handle) {
    let _ = app.dispatch_command(&format!("BEDIT_BEGIN:{}", reference.value()));
    assert!(app.tabs[app.active_tab].active_block_edit.is_some());
}

fn circle(app: &mut OpenCADStudio, x: f64) {
    let result = app.automation_op(&format!(r#"{{"op":"run","cmd":"CIRCLE {x},0 2"}}"#));
    assert_eq!(result["ok"], true, "{result}");
    app.finish_pending_history(app.active_tab);
}

fn members(app: &OpenCADStudio) -> Vec<Handle> {
    app.tabs[app.active_tab]
        .scene
        .document
        .block_records
        .get("EDIT_ME")
        .unwrap()
        .entity_handles
        .clone()
}

fn main_depth(app: &OpenCADStudio) -> usize {
    let tab = &app.tabs[app.active_tab];
    tab.block_edit_history
        .as_ref()
        .map(|h| h.main.undo_stack.len())
        .unwrap_or(tab.history.undo_stack.len())
}

#[test]
fn session_undo_preserves_membership_and_cannot_reach_drawing_history() {
    let (mut app, reference) = fixture();
    circle(&mut app, 50.0);
    let depth = main_depth(&app);
    begin(&mut app, reference);
    assert_eq!(main_depth(&app), depth);
    circle(&mut app, 1.0);
    assert_eq!(members(&app).len(), 2);
    app.undo_steps(1);
    assert_eq!(members(&app).len(), 1);
    assert!(!app.block_edit_dirty(app.active_tab));
    app.undo_steps(100);
    assert_eq!(main_depth(&app), depth);
    assert!(app.tabs[app.active_tab].active_block_edit.is_some());
    app.redo_steps(1);
    assert_eq!(members(&app).len(), 2);
    assert!(members(&app).iter().all(|h| app.tabs[app.active_tab]
        .scene
        .document
        .get_entity(*h)
        .is_some()));
}

#[test]
fn repeated_saves_stay_open_and_commit_one_drawing_step_each() {
    let (mut app, reference) = fixture();
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    circle(&mut app, 2.0);
    let _ = app.dispatch_command("BSAVE");
    assert!(app.tabs[app.active_tab].active_block_edit.is_some());
    assert_eq!(main_depth(&app), 1);
    assert!(!app.block_edit_dirty(app.active_tab));
    let _ = app.dispatch_command("BSAVE");
    assert_eq!(main_depth(&app), 1, "a clean save is not a new edit");
    circle(&mut app, 3.0);
    let _ = app.dispatch_command("BSAVE");
    assert_eq!(main_depth(&app), 2);
    circle(&mut app, 4.0);
    let _ = app.dispatch_command("BEDIT_DISCARD");
    assert_eq!(members(&app).len(), 4, "discard restores the last save");
    assert_eq!(main_depth(&app), 2);
    assert_eq!(
        app.tabs[app.active_tab]
            .history
            .undo_stack
            .last()
            .unwrap()
            .label(),
        "Edit block EDIT_ME"
    );
    app.undo_steps(1);
    assert_eq!(members(&app).len(), 3);
    app.undo_steps(1);
    assert_eq!(members(&app).len(), 1);
    app.redo_steps(2);
    assert_eq!(members(&app).len(), 4);
}

#[test]
fn discard_preserves_drawing_redo_and_never_leaks_local_edits() {
    let (mut app, reference) = fixture();
    circle(&mut app, 50.0);
    app.undo_steps(1);
    let main_redo = app.tabs[app.active_tab].history.redo_stack.len();
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    let _ = app.dispatch_command("BEDIT_DISCARD");
    assert_eq!(members(&app).len(), 1);
    assert_eq!(app.tabs[app.active_tab].history.redo_stack.len(), main_redo);
    app.redo_steps(1);
    assert_eq!(members(&app).len(), 1);
}

#[test]
fn dirty_close_cancel_discard_and_save_follow_the_checkpoint() {
    let (mut app, reference) = fixture();
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    let _ = app.dispatch_command("BCLOSE");
    assert_eq!(app.active_modal, Some(ModalKind::BlockEditChanges));
    let _ = app.update(Message::BlockEditChangesCancel);
    assert!(app.tabs[app.active_tab].active_block_edit.is_some());
    assert_eq!(members(&app).len(), 2);
    let _ = app.dispatch_command("BCLOSE");
    let _ = app.update(Message::BlockEditChangesDiscard);
    assert!(app.tabs[app.active_tab].active_block_edit.is_none());
    assert_eq!(members(&app).len(), 1);
    assert_eq!(main_depth(&app), 0);
    begin(&mut app, reference);
    circle(&mut app, 2.0);
    let _ = app.dispatch_command("BCLOSE");
    let _ = app.update(Message::BlockEditChangesSave);
    assert!(app.tabs[app.active_tab].active_block_edit.is_none());
    assert_eq!(members(&app).len(), 2);
    assert_eq!(main_depth(&app), 1);
}

#[test]
fn clean_close_does_not_prompt_or_dirty_the_drawing() {
    let (mut app, reference) = fixture();
    begin(&mut app, reference);
    assert!(!app.tabs[app.active_tab].dirty);
    let _ = app.dispatch_command("BCLOSE");
    assert!(app.tabs[app.active_tab].active_block_edit.is_none());
    assert!(app.active_modal.is_none());
    assert!(!app.tabs[app.active_tab].dirty);
    assert_eq!(main_depth(&app), 0);
}

#[test]
fn dirty_switch_can_cancel_and_never_nests_sessions() {
    let (mut app, reference) = fixture();
    let i = app.active_tab;
    app.tabs[i]
        .scene
        .define_block_raw("OTHER", Vector3::ZERO, vec![]);
    let other = app.tabs[i]
        .scene
        .document
        .add_entity(EntityType::Insert(Insert::new("OTHER", Vector3::ZERO)))
        .unwrap();
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    let _ = app.dispatch_command(&format!("BEDIT_BEGIN:{}", other.value()));
    assert_eq!(app.active_modal, Some(ModalKind::BlockEditChanges));
    let _ = app.update(Message::BlockEditChangesCancel);
    assert_eq!(
        app.tabs[i].active_block_edit_session().unwrap().block_name,
        "EDIT_ME"
    );
    let _ = app.dispatch_command(&format!("BEDIT_BEGIN:{}", other.value()));
    let _ = app.update(Message::BlockEditChangesSave);
    assert_eq!(app.tabs[i].block_edits.len(), 1);
    assert_eq!(
        app.tabs[i].active_block_edit_session().unwrap().block_name,
        "OTHER"
    );
    assert_eq!(main_depth(&app), 1);
}

#[test]
fn tab_close_prompts_for_block_edits_before_drawing_changes() {
    let (mut app, reference) = fixture();
    let id = app.tabs[app.active_tab].id;
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    let _ = app.update(Message::TabClose(id));
    assert_eq!(app.active_modal, Some(ModalKind::BlockEditChanges));
    let _ = app.update(Message::BlockEditChangesCancel);
    assert!(app.tabs.iter().any(|t| t.id == id));
    let _ = app.update(Message::TabClose(id));
    let _ = app.update(Message::BlockEditChangesDiscard);
    assert!(!app.tabs.iter().any(|t| t.id == id));
}

#[test]
fn save_refuses_a_cycle_introduced_outside_the_insert_command() {
    let (mut app, reference) = fixture();
    let i = app.active_tab;
    app.tabs[i].scene.define_block_raw(
        "OTHER",
        Vector3::ZERO,
        vec![EntityType::Insert(Insert::new("EDIT_ME", Vector3::ZERO))],
    );
    begin(&mut app, reference);
    app.tabs[i]
        .scene
        .add_entity(EntityType::Insert(Insert::new("OTHER", Vector3::ZERO)));
    assert!(!app.save_block_edit(i));
    assert_eq!(main_depth(&app), 0);
    assert!(app.tabs[i].active_block_edit.is_some());
}

#[test]
fn drawing_history_retains_the_paper_layout_after_save_and_undo() {
    let (mut app, reference) = fixture();
    let i = app.active_tab;
    app.tabs[i].scene.add_layout("Sheet").unwrap();
    app.tabs[i].scene.set_current_layout("Sheet".into());
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    let _ = app.dispatch_command("BEDIT_SAVE");
    assert_eq!(app.tabs[i].scene.current_layout, "Sheet");
    assert!(!app.tabs[i].scene.document.header.show_model_space);
    app.undo_steps(1);
    assert_eq!(app.tabs[i].scene.current_layout, "Sheet");
    assert!(!app.tabs[i].scene.document.header.show_model_space);
    assert_eq!(
        crate::io::saved_active_layout(&app.tabs[i].scene.document).as_deref(),
        Some("Sheet")
    );
    assert_eq!(members(&app).len(), 1);
    app.redo_steps(1);
    assert!(!app.tabs[i].scene.document.header.show_model_space);
    assert_eq!(members(&app).len(), 2);
}

#[test]
fn local_parameters_are_saved_and_discarded_with_the_definition() {
    let (mut app, reference) = fixture();
    begin(&mut app, reference);
    let i = app.active_tab;
    let scope = app.tabs[i].current_parametric_scope();
    app.tabs[i]
        .scene
        .parametric_constraint_set_mut(scope)
        .local_parameters
        .set("Width", "7")
        .unwrap();
    assert!(app.save_block_edit(i));
    app.tabs[i]
        .scene
        .parametric_constraint_set_mut(scope)
        .local_parameters
        .set("Width", "9")
        .unwrap();
    let _ = app.dispatch_command("BEDIT_DISCARD");
    assert_eq!(
        app.tabs[i]
            .scene
            .parametric_constraint_set(scope)
            .unwrap()
            .local_parameters
            .resolve("Width")
            .unwrap(),
        7.0
    );
    app.undo_steps(1);
    assert!(!app.tabs[i]
        .scene
        .parametric_constraint_set(scope)
        .unwrap()
        .local_parameters
        .contains("Width"));
    app.redo_steps(1);
    assert_eq!(
        app.tabs[i]
            .scene
            .parametric_constraint_set(scope)
            .unwrap()
            .local_parameters
            .resolve("Width")
            .unwrap(),
        7.0
    );
}

#[test]
fn quit_prompts_for_block_edits_before_unsaved_drawing_changes() {
    let (mut app, reference) = fixture();
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    let _ = app.request_quit_with_block_edits();
    assert_eq!(app.active_modal, Some(ModalKind::BlockEditChanges));
    let _ = app.update(Message::BlockEditChangesCancel);
    assert!(app.pending_close.is_none());
    assert!(app.tabs[app.active_tab].active_block_edit.is_some());
    let _ = app.request_quit_with_block_edits();
    let _ = app.update(Message::BlockEditChangesSave);
    assert!(app.tabs[app.active_tab].active_block_edit.is_none());
    assert_eq!(app.active_modal, Some(ModalKind::Unsaved));
    assert!(matches!(app.pending_close, Some(super::PendingClose::Quit)));
}

#[test]
fn document_replacement_is_refused_until_the_editor_is_closed() {
    let (mut app, reference) = fixture();
    begin(&mut app, reference);
    assert_eq!(app.automation_op(r#"{"op":"new"}"#)["ok"], false);
    assert!(app.tabs[app.active_tab].active_block_edit.is_some());
}

#[test]
fn erasing_the_last_member_saves_an_empty_definition_and_undo_restores_it() {
    let (mut app, reference) = fixture();
    begin(&mut app, reference);
    let i = app.active_tab;
    let line = members(&app)[0];
    app.push_undo_snapshot(i, "Erase block member");
    app.tabs[i].scene.erase_entities(&[line]);
    app.finish_pending_history(i);
    assert!(members(&app).is_empty());
    app.undo_steps(1);
    assert_eq!(members(&app), vec![line]);
    app.redo_steps(1);
    assert!(members(&app).is_empty());
    let _ = app.dispatch_command("BEDIT_SAVE");
    assert!(members(&app).is_empty());
    app.undo_steps(1);
    assert_eq!(members(&app), vec![line]);
}

#[test]
fn close_all_resumes_after_discarding_a_dirty_block() {
    let (mut app, reference) = fixture();
    let first = app.tabs[app.active_tab].id;
    app.tabs.push(super::document::DocumentTab::new_drawing(2));
    let second = app.tabs.last().unwrap().id;
    begin(&mut app, reference);
    circle(&mut app, 1.0);
    let _ = app.update(Message::DocTabCloseAll);
    assert_eq!(app.active_modal, Some(ModalKind::BlockEditChanges));
    let _ = app.update(Message::BlockEditChangesDiscard);
    assert!(!app
        .tabs
        .iter()
        .any(|tab| tab.id == first || tab.id == second));
    assert!(app.pending_tab_closes.is_empty());
}
