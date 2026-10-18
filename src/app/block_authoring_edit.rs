use super::{Message, OpenCADStudio};
use crate::command::{CadCommand, CmdResult};
use crate::scene::block_authoring::edit::{self, Part};
use codec::Handle;
use glam::DVec3;
use iced::{Point, Rectangle, Task};

const EDIT_GRIP_START: usize = usize::MAX / 4;
pub(super) fn is_editor_grip(id: usize) -> bool {
    (EDIT_GRIP_START..EDIT_GRIP_START + 16).contains(&id)
}
struct EditCommand {
    document: u64,
    definition: Handle,
    handle: Handle,
    part: Part,
    document_snapshot: codec::CadDocument,
}
impl CadCommand for EditCommand {
    fn name(&self) -> &'static str {
        "BEDITGRIP"
    }
    fn prompt(&self) -> String {
        crate::t!("Block Editor  Specify the new authoring grip location (Escape cancels):")
            .into_owned()
    }
    fn on_point(&mut self, position: DVec3) -> CmdResult {
        CmdResult::EditBlockAuthoring {
            document: self.document,
            definition: self.definition,
            handle: self.handle,
            part: self.part,
            position,
        }
    }
    fn on_preview_wires(&mut self, position: DVec3) -> Vec<crate::scene::WireModel> {
        let mut staged = self.document_snapshot.clone();
        let at = codec::types::Vector3::new(position.x, position.y, 0.0);
        if edit::move_part(&mut staged, self.definition, self.handle, self.part, at).is_err() {
            return Vec::new();
        }
        // A legacy block basepoint is represented by the definition handle.
        // Moving it materializes a native BasePointParameter with its own
        // handle, so resolve that replacement when producing the preview.
        let preview_handle = if self.handle == self.definition {
            crate::scene::block_basepoint::parameter(&staged, self.definition)
                .map(|(handle, _)| handle)
                .unwrap_or(self.handle)
        } else {
            self.handle
        };
        let Some(item) = edit::items(&staged, self.definition)
            .into_iter()
            .find(|item| item.handle == preview_handle)
        else {
            return Vec::new();
        };
        item.graphic
            .lines
            .into_iter()
            .map(|[a, b]| {
                crate::scene::WireModel::solid_f64(
                    format!("authoring-preview-{}", self.handle.value()),
                    vec![[a.x, a.y, a.z], [b.x, b.y, b.z]],
                    crate::scene::WireModel::CYAN,
                    false,
                )
            })
            .collect()
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}
impl OpenCADStudio {
    pub(super) fn commit_authoring_name(&mut self) -> Task<Message> {
        let i = self.active_tab;
        let key = crate::ui::properties::FieldKey::Geom("authoring_name");
        let Some(name) = self.tabs[i]
            .properties
            .edit_buf
            .remove(&key)
            .map(|s| s.trim().to_owned())
        else {
            return Task::none();
        };
        if name.is_empty() {
            self.command_line
                .push_error(crate::t!("Parameter name cannot be empty.").as_ref());
            return Task::none();
        }
        let Some(handle) = self.tabs[i].selected_authoring else {
            return Task::none();
        };
        let Some(session) = self.tabs[i].active_block_edit_session() else {
            return Task::none();
        };
        let definition = session.br_handle;
        let doc = &self.tabs[i].scene.document;
        if edit::items(doc, definition)
            .iter()
            .any(|item| item.handle != handle && item.name.eq_ignore_ascii_case(&name))
        {
            self.command_line
                .push_error(crate::t!("Parameter names must be unique.").as_ref());
            return Task::none();
        }
        let mut staged = doc.clone();
        let changed = match staged.objects.get_mut(&handle) {
            Some(codec::objects::ObjectType::BlockVisibilityParameter(parameter)) => {
                parameter.name = name;
                true
            }
            Some(codec::objects::ObjectType::DynamicBlock(object)) => match &mut object.data {
                codec::objects::DynamicBlockData::PointParameter(p) => {
                    p.position_name = name;
                    true
                }
                codec::objects::DynamicBlockData::LinearParameter(p) => {
                    p.distance_name = name;
                    true
                }
                codec::objects::DynamicBlockData::PolarParameter(p) => {
                    p.distance_name = name;
                    true
                }
                codec::objects::DynamicBlockData::XYParameter(p) => {
                    p.x_label = name;
                    true
                }
                codec::objects::DynamicBlockData::RotationParameter(p) => {
                    p.angle_name = name;
                    true
                }
                codec::objects::DynamicBlockData::AlignmentParameter(p) => {
                    p.parameter.parameter.element.name = name;
                    true
                }
                codec::objects::DynamicBlockData::FlipParameter(p) => {
                    p.flip_label = name;
                    true
                }
                codec::objects::DynamicBlockData::BasePointParameter(p) => {
                    p.parameter.parameter.element.name = name;
                    true
                }
                codec::objects::DynamicBlockData::LookupParameter(p) => {
                    p.lookup_name = name;
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if changed {
            if let Some(codec::objects::ObjectType::BlockVisibilityParameter(parameter)) =
                staged.objects.get(&handle)
            {
                staged
                    .block_visibility_params
                    .insert(handle, parameter.clone());
            }
            self.push_undo_snapshot(i, "Rename block parameter");
            self.tabs[i].scene.document = staged;
            self.tabs[i].dirty = true;
            self.tabs[i].scene.bump_geometry();
            self.finish_pending_history(i);
            self.refresh_properties();
        }
        Task::none()
    }
    pub(super) fn pick_authoring_box(&mut self, a: Point, b: Point, crossing: bool) -> bool {
        let i = self.active_tab;
        let Some(session) = self.tabs[i].active_block_edit_session() else {
            return false;
        };
        let items = edit::items(&self.tabs[i].scene.document, session.br_handle);
        let bounds = iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: self.tabs[i].scene.selection.borrow().view.vp_size.0,
            height: self.tabs[i].scene.selection.borrow().view.vp_size.1,
        };
        let camera = self.tabs[i].scene.camera.borrow();
        let min = Point::new(a.x.min(b.x), a.y.min(b.y));
        let max = Point::new(a.x.max(b.x), a.y.max(b.y));
        let hit = items
            .into_iter()
            .filter_map(|item| {
                let mut projected = Vec::new();
                for (_, p) in &item.grips {
                    if let Some(p) = camera.project(DVec3::new(p.x, p.y, p.z), bounds) {
                        projected.push(Point::new(p.x, p.y));
                    }
                }
                let segments = item
                    .graphic
                    .lines
                    .iter()
                    .filter_map(|[a, b]| {
                        let a = camera.project(DVec3::new(a.x, a.y, a.z), bounds)?;
                        let b = camera.project(DVec3::new(b.x, b.y, b.z), bounds)?;
                        let segment = [Point::new(a.x, a.y), Point::new(b.x, b.y)];
                        projected.extend(segment);
                        Some(segment)
                    })
                    .collect::<Vec<_>>();
                let labels = item
                    .graphic
                    .labels
                    .iter()
                    .filter_map(|(point, label)| {
                        let point =
                            camera.project(DVec3::new(point.x, point.y, point.z), bounds)?;
                        let point = Point::new(point.x, point.y);
                        projected.push(point);
                        Some((point, label.chars().count() as f32 * 7.0 + 20.0, 16.0))
                    })
                    .collect::<Vec<_>>();
                let inside =
                    |p: Point| p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y;
                let crosses = |[a, b]: [Point; 2]| {
                    if inside(a) || inside(b) {
                        return true;
                    }
                    let (dx, dy) = (b.x - a.x, b.y - a.y);
                    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
                    for (p, q) in [
                        (-dx, a.x - min.x),
                        (dx, max.x - a.x),
                        (-dy, a.y - min.y),
                        (dy, max.y - a.y),
                    ] {
                        if p.abs() <= f32::EPSILON {
                            if q < 0.0 {
                                return false;
                            }
                            continue;
                        }
                        let t = q / p;
                        if p < 0.0 {
                            enter = enter.max(t);
                        } else {
                            leave = leave.min(t);
                        }
                        if enter > leave {
                            return false;
                        }
                    }
                    true
                };
                let any = projected.iter().any(|p| inside(*p))
                    || segments.iter().copied().any(crosses)
                    || labels.iter().any(|(p, width, height)| {
                        p.x <= max.x
                            && p.x + width >= min.x
                            && p.y - height <= max.y
                            && p.y >= min.y
                    });
                let all = !projected.is_empty() && projected.iter().all(|p| inside(*p));
                (if crossing { any } else { all }).then_some(item.handle)
            })
            .next();
        drop(camera);
        if let Some(handle) = hit {
            self.tabs[i].selected_authoring = Some(handle);
            self.tabs[i].scene.deselect_all();
            self.refresh_properties();
            true
        } else {
            false
        }
    }
    pub(super) fn authoring_label_at(
        &self,
        p: Point,
        bounds: Rectangle,
    ) -> Option<(Handle, String)> {
        let tab = &self.tabs[self.active_tab];
        let session = tab.active_block_edit_session()?;
        let camera = tab.scene.camera.borrow();
        edit::items(&tab.scene.document, session.br_handle)
            .into_iter()
            .find_map(|item| {
                item.graphic.labels.iter().find_map(|(point, label)| {
                    let projected =
                        camera.project(DVec3::new(point.x, point.y, point.z), bounds)?;
                    let anchor = Point::new(projected.x, projected.y);
                    let width = (label.chars().count() as f32 * 7.0).max(20.0);
                    (p.x >= anchor.x - 8.0
                        && p.x <= anchor.x + width + 12.0
                        && (p.y - anchor.y).abs() < 16.0)
                        .then(|| (item.handle, label.clone()))
                })
            })
    }

    pub(super) fn authoring_name_is_editable(&self, handle: Handle) -> bool {
        match self.tabs[self.active_tab]
            .scene
            .document
            .objects
            .get(&handle)
        {
            Some(codec::objects::ObjectType::BlockVisibilityParameter(_)) => true,
            Some(codec::objects::ObjectType::DynamicBlock(object)) => matches!(
                object.data,
                codec::objects::DynamicBlockData::PointParameter(_)
                    | codec::objects::DynamicBlockData::LinearParameter(_)
                    | codec::objects::DynamicBlockData::PolarParameter(_)
                    | codec::objects::DynamicBlockData::XYParameter(_)
                    | codec::objects::DynamicBlockData::RotationParameter(_)
                    | codec::objects::DynamicBlockData::AlignmentParameter(_)
                    | codec::objects::DynamicBlockData::FlipParameter(_)
                    | codec::objects::DynamicBlockData::BasePointParameter(_)
                    | codec::objects::DynamicBlockData::LookupParameter(_)
            ),
            _ => false,
        }
    }

    pub(super) fn pick_authoring_object(&mut self, p: Point, bounds: Rectangle) -> bool {
        let i = self.active_tab;
        let tab = &self.tabs[i];
        if tab.active_cmd.is_some() || tab.active_grip.is_some() {
            return false;
        }
        let Some(session) = tab.active_block_edit_session() else {
            return false;
        };
        let items = edit::items(&tab.scene.document, session.br_handle);
        let camera = tab.scene.camera.borrow();
        let project = |v: codec::types::Vector3| {
            camera
                .project(DVec3::new(v.x, v.y, v.z), bounds)
                .map(|v| Point::new(v.x, v.y))
        };
        let segment = |a: Point, b: Point| {
            let ab = glam::Vec2::new(b.x - a.x, b.y - a.y);
            let ap = glam::Vec2::new(p.x - a.x, p.y - a.y);
            let t = (ap.dot(ab) / ab.length_squared().max(1e-12)).clamp(0., 1.);
            (ap - ab * t).length()
        };
        let hit = items
            .iter()
            .filter_map(|item| {
                let mut d = f32::INFINITY;
                for (_, v) in &item.grips {
                    if let Some(v) = project(*v) {
                        d = d.min(segment(v, v));
                    }
                }
                for [a, b] in &item.graphic.lines {
                    if let (Some(a), Some(b)) = (project(*a), project(*b)) {
                        d = d.min(segment(a, b));
                    }
                }
                for (v, label) in &item.graphic.labels {
                    if let Some(v) = project(*v) {
                        // Labels are drawn starting beside their anchor, in screen pixels.
                        let width = (label.chars().count() as f32 * 7.).max(20.);
                        if p.x >= v.x - 8. && p.x <= v.x + width + 12. && (p.y - v.y).abs() < 16. {
                            d = d.min(0.);
                        }
                    }
                }
                (d <= 10.).then_some((d, item.handle, item.name.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        drop(camera);
        if let Some((_, handle, name)) = hit {
            if self.shift_down && self.tabs[i].selected_authoring == Some(handle) {
                self.tabs[i].selected_authoring = None;
            } else {
                self.tabs[i].selected_authoring = Some(handle);
            }
            self.tabs[i].scene.deselect_all();
            self.tabs[i]
                .scene
                .selection
                .borrow_mut()
                .clear_left_selection_gesture();
            self.refresh_properties();
            self.command_line.push_info(&format!(
                "{} — {}",
                name,
                crate::t!("Select a grip to edit its location.")
            ));
            true
        } else {
            if self.tabs[i].selected_authoring.take().is_some() {
                self.refresh_properties();
            }
            false
        }
    }
    pub(super) fn refresh_authoring_grips(&mut self, wo: [f64; 3]) {
        let i = self.active_tab;
        let Some(handle) = self.tabs[i].selected_authoring else {
            return;
        };
        let item = self.tabs[i].active_block_edit_session().and_then(|s| {
            edit::items(&self.tabs[i].scene.document, s.br_handle)
                .into_iter()
                .find(|p| p.handle == handle)
        });
        let Some(item) = item.filter(|_| self.tabs[i].scene.selected.is_empty()) else {
            self.tabs[i].selected_authoring = None;
            return;
        };
        for (index, (_, point)) in item.grips.into_iter().enumerate() {
            self.tabs[i]
                .selected_grips
                .push(crate::scene::model::object::GripDef {
                    id: EDIT_GRIP_START + index,
                    world: DVec3::new(point.x - wo[0], point.y - wo[1], point.z - wo[2]),
                    is_midpoint: false,
                    shape: crate::scene::model::object::GripShape::Square,
                    dir: None,
                    axis: None,
                });
            self.tabs[i].selected_grip_handles.push(handle);
        }
    }
    pub(super) fn start_authoring_grip(&mut self, handle: Handle, id: usize) -> bool {
        if !is_editor_grip(id) {
            return false;
        }
        let i = self.active_tab;
        let Some(session) = self.tabs[i].active_block_edit_session() else {
            return false;
        };
        let Some(part) = edit::items(&self.tabs[i].scene.document, session.br_handle)
            .into_iter()
            .find(|p| p.handle == handle)
            .and_then(|p| p.grips.get(id - EDIT_GRIP_START).map(|(part, _)| *part))
        else {
            return false;
        };
        let command = EditCommand {
            document: self.tabs[i].id,
            definition: session.br_handle,
            handle,
            part,
            document_snapshot: self.tabs[i].scene.document.clone(),
        };
        self.command_line.push_info(&command.prompt());
        self.tabs[i].active_cmd = Some(Box::new(command));
        self.grip_hover = None;
        self.grip_popup = None;
        true
    }
    pub(super) fn handle_authoring_edit(&mut self, result: CmdResult) -> Task<Message> {
        let CmdResult::EditBlockAuthoring {
            document,
            definition,
            handle,
            part,
            position,
        } = result
        else {
            unreachable!()
        };
        let i = self.active_tab;
        if self.tabs[i].id != document
            || self.tabs[i]
                .active_block_edit_session()
                .map(|s| s.br_handle)
                != Some(definition)
        {
            self.command_line
                .push_error(crate::t!("The Block Editor session is no longer active.").as_ref());
            return Task::none();
        }
        let mut staged = self.tabs[i].scene.document.clone();
        match edit::move_part(
            &mut staged,
            definition,
            handle,
            part,
            codec::types::Vector3::new(position.x, position.y, position.z),
        ) {
            Ok(changed) => {
                if changed {
                    self.push_undo_snapshot(i, "Block authoring grip");
                    // Keep the live entity-change recorder attached. Basepoint edits
                    // can also relocate INSERT attributes outside the edited block.
                    let doc = &mut self.tabs[i].scene.document;
                    let changed_entities = staged
                        .entities()
                        .filter(|e| doc.get_entity(e.common().handle) != Some(*e))
                        .cloned()
                        .collect::<Vec<_>>();
                    for entity in changed_entities {
                        let handle = entity.common().handle;
                        *doc.get_entity_mut(handle)
                            .expect("authoring edits retain entity identities") = entity;
                    }
                    for (h, o) in staged.objects.iter() {
                        if doc.objects.get(h) != Some(o) {
                            doc.objects.insert(*h, o.clone());
                        }
                    }
                    doc.block_visibility_params = staged.block_visibility_params.clone();
                    self.tabs[i].scene.bump_geometry();
                    self.tabs[i].dirty = true;
                    self.finish_pending_history(i);
                }
                self.tabs[i].active_cmd = None;
                self.tabs[i].scene.clear_preview_wire();
                self.refresh_properties();
            }
            Err(e) => self.command_line.push_error(crate::t!(e).as_ref()),
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_basepoint_preview_follows_materialized_parameter() {
        let mut scene = crate::scene::Scene::new();
        scene.define_block_raw(
            "PREVIEW_BASEPOINT",
            codec::types::Vector3::new(2.0, 3.0, 0.0),
            vec![codec::EntityType::Line(codec::entities::Line::from_points(
                codec::types::Vector3::ZERO,
                codec::types::Vector3::new(1.0, 0.0, 0.0),
            ))],
        );
        let definition = scene
            .document
            .block_records
            .get("PREVIEW_BASEPOINT")
            .unwrap()
            .handle;
        assert!(crate::scene::block_basepoint::parameter(&scene.document, definition).is_none());
        let mut command = EditCommand {
            document: 1,
            definition,
            handle: definition,
            part: Part::Point,
            document_snapshot: scene.document.clone(),
        };

        let preview = command.on_preview_wires(DVec3::new(4.0, 5.0, 0.0));

        assert!(!preview.is_empty());
    }
}
