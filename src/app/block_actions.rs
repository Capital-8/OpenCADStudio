use super::{Message, OpenCADStudio};
use crate::command::CadCommand;
use crate::scene::block_actions::{self as actions, Kind};
use codec::{EntityType, Handle};
use iced::Task;

// These grips start an ordinary point-input command, keeping snapping, typed
// coordinates and Escape on the command driver's existing path.
const TEST_GRIP_START: usize = usize::MAX / 2;
pub(super) fn is_test_grip(id: usize) -> bool {
    (TEST_GRIP_START..usize::MAX).contains(&id)
}
impl OpenCADStudio {
    pub(in crate::app) fn action_error(&self, kind: Option<Kind>) -> Option<&'static str> {
        let tab = &self.tabs[self.active_tab];
        let Some(s) = tab.active_block_edit_session() else {
            return Some("Open a block in the Block Editor before adding actions.");
        };
        if let Some(e) =
            crate::scene::block_point_parameter::placement_error(&tab.scene.document, s.br_handle)
        {
            return Some(e);
        }
        let candidates = kind.map_or_else(
            || actions::all_parameters(&tab.scene.document, s.br_handle),
            |kind| actions::parameters(&tab.scene.document, s.br_handle, kind),
        );
        if candidates.is_empty() {
            return Some("Place a compatible parameter before adding this action.");
        }
        None
    }
    pub(in crate::app) fn action_set_error(&self) -> Option<&'static str> {
        let tab = &self.tabs[self.active_tab];
        let Some(session) = tab.active_block_edit_session() else {
            return Some("BACTIONSET is only available in the Block Editor.");
        };
        if let Some(e) = crate::scene::block_point_parameter::placement_error(
            &tab.scene.document,
            session.br_handle,
        ) {
            return Some(e);
        }
        actions::actions(&tab.scene.document, session.br_handle)
            .is_empty()
            .then_some("This block has no supported actions to edit.")
    }
    pub(in crate::app) fn start_block_action_set(&mut self) -> Task<Message> {
        if let Some(e) = self.action_set_error() {
            self.command_line.push_error(crate::t!(e).as_ref());
            return Task::none();
        }
        let tab = &self.tabs[self.active_tab];
        let command = crate::modules::block_editor::action_set::ActionSetCommand::new(
            tab.id,
            tab.active_block_edit_session().unwrap().br_handle,
            &tab.scene.document,
            tab.selected_authoring,
        );
        self.command_line.push_info(&command.prompt());
        self.command_line.set_step_options(command.options());
        self.tabs[self.active_tab].active_cmd = Some(Box::new(command));
        Task::none()
    }
    fn apply_action_document(&mut self, i: usize, mut staged: codec::CadDocument) {
        let doc = &mut self.tabs[i].scene.document;
        let changes = staged
            .entities()
            .filter(|e| doc.get_entity(e.common().handle) != Some(*e))
            .cloned()
            .collect::<Vec<_>>();
        doc.swap_structure(staged.snapshot_structure());
        for entity in changes {
            let handle = entity.common().handle;
            if let Some(current) = doc.get_entity_mut(handle) {
                *current = entity;
            } else {
                doc.restore_entity_arc(std::sync::Arc::new(entity));
            }
        }
    }
    pub(in crate::app) fn start_block_action(&mut self, args: &str) -> Task<Message> {
        let kind = if args.is_empty() {
            None
        } else {
            Kind::parse(args)
        };
        if !args.is_empty() && kind.is_none() {
            self.command_line.push_error(
                crate::t!("Supported actions: Move, Scale, Stretch, Rotate, Flip. Other action types are not wired yet.")
                    .as_ref(),
            );
            return Task::none();
        }
        if let Some(e) = self.action_error(kind) {
            self.command_line.push_error(crate::t!(e).as_ref());
            return Task::none();
        }
        let tab = &self.tabs[self.active_tab];
        let definition = tab.active_block_edit_session().unwrap().br_handle;
        let command = crate::modules::block_editor::action::ActionCommand::new(
            tab.id,
            definition,
            kind,
            kind.map_or_else(
                || actions::all_parameters(&tab.scene.document, definition),
                |kind| actions::parameters(&tab.scene.document, definition, kind),
            ),
        );
        self.command_line.push_info(&command.prompt());
        self.command_line.set_step_options(command.options());
        self.tabs[self.active_tab].active_cmd = Some(Box::new(command));
        Task::none()
    }
    pub(in crate::app) fn handle_block_action_result(
        &mut self,
        result: crate::command::CmdResult,
    ) -> Task<Message> {
        let i = self.active_tab;
        match result {
            crate::command::CmdResult::EditBlockActionSet {
                document,
                definition,
                action,
                expected,
                targets,
                frame,
            } => {
                let tab = &self.tabs[i];
                if tab.id != document
                    || tab.active_block_edit_session().map(|s| s.br_handle) != Some(definition)
                    || tab.scene.document.objects.get(&action) != Some(expected.as_ref())
                {
                    self.command_line.push_error(
                        crate::t!(
                            "The action or its Block Editor session changed; restart BACTIONSET."
                        )
                        .as_ref(),
                    );
                    return Task::none();
                }
                let mut staged = tab.scene.document.clone();
                let result = actions::retarget(&mut staged, definition, action, &targets, frame)
                    .and_then(|changed| {
                        if changed {
                            staged = actions::instance::refresh(&staged, definition)?;
                        }
                        Ok(changed)
                    });
                match result {
                    Ok(changed) => {
                        if changed {
                            self.push_undo_snapshot(i, "Action selection set");
                            self.apply_action_document(i, staged);
                            self.tabs[i].scene.bump_geometry();
                            self.tabs[i].dirty = true;
                            self.finish_pending_history(i);
                        }
                        self.tabs[i].active_cmd = None;
                        self.tabs[i].scene.clear_preview_wire();
                        self.refresh_properties();
                        self.command_line
                            .push_output(crate::t!("Action selection updated.").as_ref());
                    }
                    Err(e) => self.command_line.push_error(crate::t!(e).as_ref()),
                }
            }
            crate::command::CmdResult::PlaceBlockAction {
                document,
                definition,
                kind,
                parameter,
                targets,
                location,
                frame,
            } => {
                if self.tabs[i].id != document
                    || self.tabs[i]
                        .active_block_edit_session()
                        .map(|s| s.br_handle)
                        != Some(definition)
                {
                    self.command_line.push_error(
                        crate::t!("The action's Block Editor session is no longer active.")
                            .as_ref(),
                    );
                    return Task::none();
                }
                let mut staged = self.tabs[i].scene.document.clone();
                match actions::create_with_frame(
                    &mut staged,
                    definition,
                    kind,
                    parameter,
                    &targets,
                    location,
                    frame,
                ) {
                    Ok(_) => {
                        self.push_undo_snapshot(i, &format!("{} action", kind.name()));
                        self.tabs[i].scene.document = staged;
                        self.tabs[i].scene.bump_geometry();
                        self.tabs[i].dirty = true;
                        self.tabs[i].active_cmd = None;
                        self.tabs[i].scene.clear_preview_wire();
                        self.finish_pending_history(i);
                        self.refresh_properties();
                        self.command_line.push_output(crate::tf!("{} action attached. Select the reference and use its parameter grip, or open Test Block.",crate::t!(kind.name())).as_ref());
                    }
                    Err(e) => self.command_line.push_error(crate::t!(e).as_ref()),
                }
            }
            crate::command::CmdResult::EvaluateBlockParameter {
                insert,
                document,
                definition,
                parameter,
                value,
            } => {
                if self.tabs[i].id != document {
                    return Task::none();
                }
                if let Some(insert) = insert {
                    return self.evaluate_reference_parameter(insert, definition, parameter, value);
                }
                let Some(test) = self.tabs[i].block_test.as_ref() else {
                    return Task::none();
                };
                if test.definition != definition {
                    return Task::none();
                }
                let Some(EntityType::Insert(insert)) =
                    self.tabs[i].scene.document.get_entity(test.insert)
                else {
                    return Task::none();
                };
                let doc = &self.tabs[i].scene.document;
                if doc.entities().any(|e|matches!(e,EntityType::Insert(other) if other.common.handle!=test.insert && (other.block_name==insert.block_name||doc.dynamic_definition_for_insert(other.common.handle)==Some(definition)))) {
                    self.command_line.push_error(crate::t!("Test actions with one reference in Test Block. Remove the extra references or reopen Test Block.").as_ref());return Task::none();
                }
                let Some(inverse) =
                    crate::scene::inverse_affine(&crate::scene::render_graph::insert_transform(
                        &self.tabs[i].scene.document,
                        insert,
                    ))
                else {
                    self.command_line.push_error(
                        crate::t!("The test reference has a singular transform.").as_ref(),
                    );
                    return Task::none();
                };
                let local = match value {
                    actions::Value::Point(at) => {
                        actions::Value::Point(actions::v(inverse.apply(actions::native(at))))
                    }
                    state => state,
                };
                let representation = insert.block_name.clone();
                let mut staged = self.tabs[i].scene.document.clone();
                match actions::evaluate_value(
                    &mut staged,
                    &test.baseline,
                    definition,
                    parameter,
                    local,
                ) {
                    Ok(changed) => {
                        if changed {
                            if let Err(e) = actions::sync_representation(
                                &mut staged,
                                definition,
                                &representation,
                            ) {
                                self.command_line.push_error(crate::t!(e).as_ref());
                                return Task::none();
                            }
                            self.finish_pending_history(i);
                            let before = &self.tabs[i].scene.document;
                            let entities = before
                                .entities()
                                .filter_map(|e| {
                                    let h = e.common().handle;
                                    (staged.get_entity(h) != Some(e))
                                        .then(|| (h, before.get_entity_arc(h).unwrap()))
                                })
                                .collect();
                            let objects = before
                                .objects
                                .iter()
                                .filter(|(h, o)| staged.objects.get(h) != Some(*o))
                                .map(|(h, o)| (*h, o.clone()))
                                .collect();
                            let dirty_before = self.tabs[i].dirty;
                            self.tabs[i].scene.document = staged;
                            self.tabs[i].scene.bump_geometry();
                            self.tabs[i].dirty = true;
                            self.push_entity_group_history(
                                i,
                                "Test block parameter",
                                entities,
                                objects,
                                dirty_before,
                            );
                        }
                        self.tabs[i].active_cmd = None;
                        self.tabs[i].scene.clear_preview_wire();
                        self.refresh_properties();
                    }
                    Err(e) => self.command_line.push_error(crate::t!(e).as_ref()),
                }
            }
            _ => unreachable!(),
        }
        Task::none()
    }
    pub(super) fn refresh_test_action_grips(&mut self, wo: [f64; 3]) {
        let i = self.active_tab;
        let tab = &self.tabs[i];
        let Some(test) = tab.block_test.as_ref() else {
            self.refresh_reference_action_grips(wo);
            return;
        };
        if tab.selected_handle != Some(test.insert) {
            return;
        }
        let Some(EntityType::Insert(insert)) = tab.scene.document.get_entity(test.insert) else {
            return;
        };
        let transform = crate::scene::render_graph::insert_transform(&tab.scene.document, insert);
        let mut grips = vec![];
        let all = actions::actions(&test.baseline, test.definition);
        for (index, p) in actions::all_parameters(&test.baseline, test.definition)
            .into_iter()
            .enumerate()
        {
            if !all.iter().any(|(_, _, _, id)| *id == p.id) {
                continue;
            }
            let Ok(value) = actions::value(&tab.scene.document, &p) else {
                continue;
            };
            let world =
                actions::v(transform.apply(actions::native(actions::grip_position(&p, value))))
                    - glam::DVec3::from_array(wo);
            grips.push(crate::scene::model::object::GripDef {
                id: TEST_GRIP_START + index,
                world,
                is_midpoint: false,
                shape: crate::scene::model::object::GripShape::Triangle,
                dir: actions::grip_direction(&p, value).and_then(|direction| {
                    (actions::v(transform.apply(actions::native(direction)))
                        - actions::v(transform.apply(codec::types::Vector3::ZERO)))
                    .try_normalize()
                }),
                axis: None,
            });
        }
        let handle = test.insert;
        let tab = &mut self.tabs[i];
        for grip in grips {
            tab.selected_grips.push(grip);
            tab.selected_grip_handles.push(handle);
        }
    }
    pub(super) fn start_test_action_grip(&mut self, handle: Handle, id: usize) -> bool {
        if !is_test_grip(id) {
            return false;
        }
        let i = self.active_tab;
        let tab = &self.tabs[i];
        let Some(test) = tab.block_test.as_ref() else {
            return self.start_reference_action_grip(handle, id);
        };
        if handle != test.insert {
            return false;
        }
        if tab.scene.is_layer_locked(handle) {
            return true;
        }
        let Some(p) = actions::all_parameters(&test.baseline, test.definition)
            .get(id - TEST_GRIP_START)
            .cloned()
        else {
            return false;
        };
        if p.flip {
            let state = actions::value(&tab.scene.document, &p);
            let document = tab.id;
            let definition = test.definition;
            return self.toggle_action_grip(document, definition, p.handle, None, state);
        }
        let command = crate::modules::block_editor::action::TestParameterCommand {
            insert: None,
            document: tab.id,
            definition: test.definition,
            parameter: p.handle,
        };
        self.command_line.push_info(&command.prompt());
        self.tabs[i].active_cmd = Some(Box::new(command));
        self.grip_hover = None;
        self.grip_popup = None;
        true
    }
}

impl OpenCADStudio {
    fn refresh_reference_action_grips(&mut self, wo: [f64; 3]) {
        let i = self.active_tab;
        let tab = &self.tabs[i];
        if tab.active_block_edit_session().is_some() || tab.scene.selected.len() != 1 {
            return;
        }
        let Some(handle) = tab.selected_handle else {
            return;
        };
        let Some(def) = actions::instance::definition(&tab.scene.document, handle) else {
            return;
        };
        let Some(EntityType::Insert(reference)) = tab.scene.document.get_entity(handle) else {
            return;
        };
        let transform =
            crate::scene::render_graph::insert_transform(&tab.scene.document, reference);
        let all = actions::actions(&tab.scene.document, def);
        let mut grips = vec![];
        for (index, p) in actions::all_parameters(&tab.scene.document, def)
            .into_iter()
            .enumerate()
        {
            if !all.iter().any(|(_, _, _, id)| *id == p.id) {
                continue;
            }
            let Ok(value) = actions::instance::value(&tab.scene.document, handle, &p) else {
                continue;
            };
            let world =
                actions::v(transform.apply(actions::native(actions::grip_position(&p, value))))
                    - glam::DVec3::from_array(wo);
            grips.push(crate::scene::model::object::GripDef {
                id: TEST_GRIP_START + index,
                world,
                is_midpoint: false,
                shape: crate::scene::model::object::GripShape::Triangle,
                dir: actions::grip_direction(&p, value).and_then(|direction| {
                    (actions::v(transform.apply(actions::native(direction)))
                        - actions::v(transform.apply(codec::types::Vector3::ZERO)))
                    .try_normalize()
                }),
                axis: None,
            });
        }
        for grip in grips {
            self.tabs[i].selected_grips.push(grip);
            self.tabs[i].selected_grip_handles.push(handle);
        }
    }
    fn start_reference_action_grip(&mut self, handle: Handle, id: usize) -> bool {
        let i = self.active_tab;
        let tab = &self.tabs[i];
        if tab.active_block_edit_session().is_some() || tab.selected_handle != Some(handle) {
            return false;
        }
        let Some(def) = actions::instance::definition(&tab.scene.document, handle) else {
            return false;
        };
        if tab.scene.is_layer_locked(handle) {
            return true;
        }
        let Some(p) = actions::all_parameters(&tab.scene.document, def)
            .get(id - TEST_GRIP_START)
            .cloned()
        else {
            return false;
        };
        if p.flip {
            let state = actions::instance::value(&tab.scene.document, handle, &p);
            let document = tab.id;
            return self.toggle_action_grip(document, def, p.handle, Some(handle), state);
        }
        let command = crate::modules::block_editor::action::TestParameterCommand {
            insert: Some(handle),
            document: tab.id,
            definition: def,
            parameter: p.handle,
        };
        self.command_line.push_info(&command.prompt());
        self.tabs[i].active_cmd = Some(Box::new(command));
        self.grip_hover = None;
        self.grip_popup = None;
        true
    }
    fn evaluate_reference_parameter(
        &mut self,
        handle: Handle,
        definition: Handle,
        parameter: Handle,
        value: actions::Value,
    ) -> Task<Message> {
        let i = self.active_tab;
        let tab = &self.tabs[i];
        if tab.active_block_edit_session().is_some()
            || tab.block_test.is_some()
            || actions::instance::definition(&tab.scene.document, handle) != Some(definition)
        {
            return Task::none();
        }
        if tab.scene.is_layer_locked(handle) {
            return Task::none();
        }
        let Some(EntityType::Insert(reference)) = tab.scene.document.get_entity(handle) else {
            return Task::none();
        };
        let Some(inverse) = crate::scene::inverse_affine(
            &crate::scene::render_graph::insert_transform(&tab.scene.document, reference),
        ) else {
            return Task::none();
        };
        let local = match value {
            actions::Value::Point(at) => {
                actions::Value::Point(actions::v(inverse.apply(actions::native(at))))
            }
            state => state,
        };
        match actions::instance::evaluate_value(&tab.scene.document, handle, parameter, local) {
            Ok(staged) => {
                if let Some(staged) = staged {
                    self.push_undo_snapshot(i, "Block reference parameter");
                    self.apply_action_document(i, staged);
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

impl OpenCADStudio {
    fn toggle_action_grip(
        &mut self,
        document: u64,
        definition: Handle,
        parameter: Handle,
        insert: Option<Handle>,
        state: Result<actions::Value, &'static str>,
    ) -> bool {
        match state {
            Ok(actions::Value::Flip(state)) => {
                let _ = self.handle_block_action_result(
                    crate::command::CmdResult::EvaluateBlockParameter {
                        document,
                        definition,
                        parameter,
                        insert,
                        value: actions::Value::Flip(!state),
                    },
                );
            }
            Err(e) => self.command_line.push_error(crate::t!(e).as_ref()),
            _ => return false,
        }
        self.grip_hover = None;
        self.grip_popup = None;
        true
    }
}
