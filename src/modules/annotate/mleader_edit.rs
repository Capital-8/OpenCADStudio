// MLEADEREDIT / AIMLEADEREDITADD / AIMLEADEREDITREMOVE / MLEADERALIGN /
// MLEADERCOLLECT commands.
//
// MLEADEREDIT:   pick a multileader → add leaders at arrowhead points, or
//                [Remove leaders] picked leaders; Enter applies the edit.
// MLEADERALIGN:  select multileaders → pick base alignment direction
// MLEADERCOLLECT: select block-content multileaders → pick collection point

use codec::entities::{LeaderLine, MultiLeader};
use codec::types::Vector3;
use codec::{EntityType, Handle};
use glam::{DVec3, Vec3};
use kernel::geom2d::{closest_point, Curve, Line};

use crate::command::{CadCommand, CmdOption, CmdResult, InputKind};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::scene::model::wire_model::WireModel;
use crate::t;

// ── MLEADEREDIT ───────────────────────────────────────────────────────────────

pub const ICON_ADD: IconKind =
    IconKind::Svg(include_bytes!("../../../assets/icons/mleader_add.svg"));

pub fn tool_add() -> ToolDef {
    ToolDef {
        id: "AIMLEADEREDITADD",
        label: "Add Leader",
        icon: ICON_ADD,
        event: ModuleEvent::Command("AIMLEADEREDITADD".to_string()),
    }
}

pub const ICON_REMOVE: IconKind =
    IconKind::Svg(include_bytes!("../../../assets/icons/mleader_remove.svg"));

pub fn tool_remove() -> ToolDef {
    ToolDef {
        id: "AIMLEADEREDITREMOVE",
        label: "Remove Leader",
        icon: ICON_REMOVE,
        event: ModuleEvent::Command("AIMLEADEREDITREMOVE".to_string()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditMode {
    Add,
    Remove,
}

/// One command behind MLEADEREDIT and its add / remove entry points: the
/// picked multileader is edited in a copy and written back in one step.
pub struct MLeaderEditCommand {
    name: &'static str,
    mode: EditMode,
    picked: Option<EntityType>,
    target: Option<(Handle, MultiLeader)>,
    /// Leader lines picked for removal, as (root, line) indices.
    removals: Vec<(usize, usize)>,
    changed: bool,
}

impl MLeaderEditCommand {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            mode: if name == "AIMLEADEREDITREMOVE" {
                EditMode::Remove
            } else {
                EditMode::Add
            },
            picked: None,
            target: None,
            removals: Vec::new(),
            changed: false,
        }
    }

    /// The leader line nearest `pick`, when the pick lands on it.
    fn leader_at(ml: &MultiLeader, pick: Vector3) -> Option<(usize, usize)> {
        let tolerance = (ml.context.arrowhead_size.max(ml.arrowhead_size) * 1.5).max(1.0e-6);
        ml.context
            .leader_roots
            .iter()
            .enumerate()
            .flat_map(|(root_index, root)| {
                root.lines.iter().enumerate().map(move |(line_index, line)| {
                    let mut distance = line
                        .points
                        .windows(2)
                        .map(|segment| point_segment_distance_xy(pick, segment[0], segment[1]))
                        .fold(f64::INFINITY, f64::min);
                    if let Some(last) = line.points.last().copied() {
                        distance = distance.min(point_segment_distance_xy(
                            pick,
                            last,
                            root.connection_point,
                        ));
                    }
                    (root_index, line_index, distance)
                })
            })
            .filter(|(_, _, distance)| *distance <= tolerance)
            .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(root, line, _)| (root, line))
    }

    /// The edited multileader with the picked leaders taken out.
    fn applied(&self) -> Option<(Handle, MultiLeader)> {
        let (handle, ml) = self.target.as_ref()?;
        let mut ml = ml.clone();
        let mut removals = self.removals.clone();
        removals.sort_unstable_by(|a, b| b.cmp(a));
        for (root, line) in removals {
            if let Some(r) = ml.context.leader_roots.get_mut(root) {
                if line < r.lines.len() {
                    r.lines.remove(line);
                }
            }
        }
        ml.context.leader_roots.retain(|root| !root.lines.is_empty());
        Some((*handle, ml))
    }
}

impl CadCommand for MLeaderEditCommand {
    fn name(&self) -> &'static str {
        self.name
    }

    fn prompt(&self) -> String {
        if self.target.is_none() {
            return crate::tr!("mleader-edit", "select");
        }
        match self.mode {
            EditMode::Add => crate::tr!("mleader-edit", "add"),
            EditMode::Remove => crate::tr!("mleader-edit", "remove"),
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        match (self.target.is_some(), self.mode) {
            (false, _) => Vec::new(),
            (true, EditMode::Add) => vec![CmdOption::new("Remove leaders", "R")],
            (true, EditMode::Remove) => vec![CmdOption::new("Add leaders", "A")],
        }
    }

    fn needs_entity_pick(&self) -> bool {
        self.target.is_none()
    }

    fn inject_before_entity_pick(&self) -> bool {
        true
    }

    fn typed_point_picks_entity(&self) -> bool {
        self.target.is_none()
    }

    fn inject_picked_entity(&mut self, entity: EntityType) {
        self.picked = Some(entity);
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        match self.picked.take() {
            Some(EntityType::MultiLeader(ml)) if !handle.is_null() => {
                self.target = Some((handle, *ml));
                CmdResult::NeedPoint
            }
            Some(_) => CmdResult::ReportError(crate::tr!("mleader-edit", "not-multileader")),
            None => CmdResult::NeedPoint,
        }
    }

    fn input_kind(&self) -> InputKind {
        InputKind::Point
    }

    fn point_step_accepts_keywords(&self) -> bool {
        self.target.is_some()
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        self.target.as_ref()?;
        let key = text.trim().trim_start_matches('_').to_ascii_uppercase();
        match (self.mode, key.as_str()) {
            (EditMode::Add, "R" | "REMOVE" | "REMOVE LEADERS") => self.mode = EditMode::Remove,
            (EditMode::Remove, "A" | "ADD" | "ADD LEADERS") => self.mode = EditMode::Add,
            _ => {
                return Some(CmdResult::ReportError(
                    t!("Invalid option keyword.").into_owned(),
                ))
            }
        }
        Some(CmdResult::NeedPoint)
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        let pick = Vector3::new(pt.x, pt.y, pt.z);
        let mode = self.mode;
        let Some((_, ml)) = self.target.as_mut() else {
            return CmdResult::NeedPoint;
        };
        match mode {
            EditMode::Add => {
                crate::entities::multileader::add_leader_line(ml, pick);
                self.changed = true;
            }
            EditMode::Remove => {
                if let Some(found) = Self::leader_at(ml, pick) {
                    if !self.removals.contains(&found) {
                        self.removals.push(found);
                        self.changed = true;
                    }
                }
            }
        }
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        if !self.changed {
            return CmdResult::Cancel;
        }
        let Some((handle, ml)) = self.applied() else {
            return CmdResult::Cancel;
        };
        // Taking out every leader leaves the content on its own.
        let message = (!self.removals.is_empty() && ml.total_leader_line_count() == 0)
            .then(|| crate::tr!("mleader-edit", "none-left"));
        let entity = EntityType::MultiLeader(Box::new(ml));
        match message {
            Some(message) => CmdResult::UpdateEntityAndReport {
                handle,
                entity,
                message,
            },
            None => CmdResult::UpdateEntityAndFinish { handle, entity },
        }
    }

    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        let (_, ml) = self.applied()?;
        // Rubber band from the cursor to the landing it would join.
        if self.mode != EditMode::Add {
            return None;
        }
        let mut probe = ml;
        crate::entities::multileader::add_leader_line(
            &mut probe,
            Vector3::new(pt.x, pt.y, pt.z),
        );
        let root = probe
            .context
            .leader_roots
            .iter()
            .find(|root| {
                root.lines
                    .last()
                    .and_then(|line| line.points.first())
                    .is_some_and(|p| (p.x - pt.x).abs() < 1e-9 && (p.y - pt.y).abs() < 1e-9)
            })?;
        let cp = root.connection_point;
        Some(preview_wire(&[
            pt.as_vec3(),
            Vec3::new(cp.x as f32, cp.y as f32, cp.z as f32),
        ]))
    }

    fn persistent_preview_wires(&self) -> Vec<WireModel> {
        // The edit so far: the added and the remaining leaders.
        let Some((_, ml)) = self.applied() else {
            return Vec::new();
        };
        ml.context
            .leader_roots
            .iter()
            .flat_map(|root| {
                root.lines.iter().map(move |line| {
                    let mut pts: Vec<Vec3> = line
                        .points
                        .iter()
                        .map(|p| Vec3::new(p.x as f32, p.y as f32, p.z as f32))
                        .collect();
                    let cp = root.connection_point;
                    pts.push(Vec3::new(cp.x as f32, cp.y as f32, cp.z as f32));
                    preview_wire(&pts)
                })
            })
            .collect()
    }
}

// ── MLEADERALIGN ─────────────────────────────────────────────────────────────

pub const ICON_ALIGN: IconKind =
    IconKind::Svg(include_bytes!("../../../assets/icons/mleader_align.svg"));

pub fn tool_align() -> ToolDef {
    ToolDef {
        id: "MLEADERALIGN",
        label: "Align Leaders",
        icon: ICON_ALIGN,
        event: ModuleEvent::Command("MLEADERALIGN".to_string()),
    }
}

enum AlignStep {
    Gathering,
    PickDirection { handles: Vec<Handle> },
    PickEndDir { handles: Vec<Handle>, from: DVec3 },
}

pub struct MLeaderAlignCommand {
    step: AlignStep,
}

impl MLeaderAlignCommand {
    pub fn new() -> Self {
        Self {
            step: AlignStep::Gathering,
        }
    }
}

impl CadCommand for MLeaderAlignCommand {
    fn name(&self) -> &'static str {
        "MLEADERALIGN"
    }

    fn prompt(&self) -> String {
        match &self.step {
            AlignStep::Gathering => {
                t!("MLEADERALIGN  Select multileaders to align (Enter when done):").into_owned()
            }
            AlignStep::PickDirection { .. } => {
                t!("MLEADERALIGN  Specify direction — pick start point:").into_owned()
            }
            AlignStep::PickEndDir { .. } => {
                t!("MLEADERALIGN  Specify end point of alignment direction:").into_owned()
            }
        }
    }

    fn is_selection_gathering(&self) -> bool {
        matches!(self.step, AlignStep::Gathering)
    }

    fn on_selection_complete(&mut self, handles: Vec<Handle>) -> CmdResult {
        if handles.is_empty() {
            return CmdResult::Cancel;
        }
        self.step = AlignStep::PickDirection { handles };
        CmdResult::NeedPoint
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match &mut self.step {
            AlignStep::PickDirection { handles } => {
                let h = handles.clone();
                self.step = AlignStep::PickEndDir {
                    handles: h,
                    from: pt,
                };
                CmdResult::NeedPoint
            }
            AlignStep::PickEndDir { handles, from } => CmdResult::AlignMLeaders {
                handles: handles.clone(),
                from: *from,
                to: pt,
            },
            _ => CmdResult::NeedPoint,
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, _pt: DVec3) -> Option<WireModel> {
        None
    }
}

// ── MLEADERCOLLECT ────────────────────────────────────────────────────────────

pub const ICON_COLLECT: IconKind =
    IconKind::Svg(include_bytes!("../../../assets/icons/mleader_collect.svg"));

pub fn tool_collect() -> ToolDef {
    ToolDef {
        id: "MLEADERCOLLECT",
        label: "Collect Leaders",
        icon: ICON_COLLECT,
        event: ModuleEvent::Command("MLEADERCOLLECT".to_string()),
    }
}

enum CollectStep {
    Gathering,
    PickLocation { handles: Vec<Handle> },
}

pub struct MLeaderCollectCommand {
    step: CollectStep,
}

impl MLeaderCollectCommand {
    pub fn new() -> Self {
        Self {
            step: CollectStep::Gathering,
        }
    }
}

impl CadCommand for MLeaderCollectCommand {
    fn name(&self) -> &'static str {
        "MLEADERCOLLECT"
    }

    fn prompt(&self) -> String {
        match &self.step {
            CollectStep::Gathering => {
                t!("MLEADERCOLLECT  Select multileaders to collect (Enter when done):").into_owned()
            }
            CollectStep::PickLocation { .. } => {
                t!("MLEADERCOLLECT  Specify collected multileader location:").into_owned()
            }
        }
    }

    fn is_selection_gathering(&self) -> bool {
        matches!(self.step, CollectStep::Gathering)
    }

    fn on_selection_complete(&mut self, handles: Vec<Handle>) -> CmdResult {
        if handles.is_empty() {
            return CmdResult::Cancel;
        }
        self.step = CollectStep::PickLocation { handles };
        CmdResult::NeedPoint
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        if let CollectStep::PickLocation { handles } = &self.step {
            return CmdResult::CollectMLeaders {
                handles: handles.clone(),
                point: pt,
            };
        }
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, _pt: DVec3) -> Option<WireModel> {
        None
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn preview_wire(pts: &[Vec3]) -> WireModel {
    WireModel {
        bg_adapt: None,
        point_marker: None,
        taper_widths: Vec::new(),
        pattern_stations: Vec::new(),
        world_width: 0.0,
        depth_override: None,
        display_visible: true,
        snap_only: false,
        plot_visible: true,
        fill_is_3d: false,
        fill_is_2d_solid: false,
        render_instance: None,
        pick_tris: Vec::new(),
        pick_tris_low: Vec::new(),
        dash_from_start: false,
        dash_align_end: None,
        text_verts: Vec::new(),
        name: "mleader_edit_preview".into(),
        points: pts.iter().map(|p| [p.x, p.y, p.z]).collect(),
        points_low: Vec::new(),
        color: WireModel::CYAN,
        selected: false,
        pattern_length: 0.0,
        pattern: [0.0; 8],
        line_weight_px: 1.0,
        snap_pts: vec![],
        tangent_geoms: vec![],
        aci: 0,
        key_vertices: vec![],
        aabb: WireModel::UNBOUNDED_AABB,
        plinegen: true,
        fill_tris: vec![],
        fill_tris_low: Vec::new(),

        ..Default::default()
    }
}

fn point_segment_distance_xy(point: Vector3, start: Vector3, end: Vector3) -> f64 {
    closest_point(
        &Curve::Line(Line {
            start: [start.x, start.y],
            end: [end.x, end.y],
        }),
        [point.x, point.y],
    )
    .distance
}

// Silence unused-import warning for MultiLeader and LeaderLine if not used in all paths
fn _uses_ml_types(_ml: &MultiLeader, _ll: &LeaderLine) {}

// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration {
    names: &["MLEADEREDIT", "AIMLEADEREDITADD", "AIMLEADEREDITREMOVE"]
}); // MLeaderEditCommand
inventory::submit!(crate::command::CommandRegistration {
    names: &["MLEADERALIGN"]
}); // MLeaderAlignCommand
inventory::submit!(crate::command::CommandRegistration {
    names: &["MLEADERCOLLECT"]
}); // MLeaderCollectCommand
