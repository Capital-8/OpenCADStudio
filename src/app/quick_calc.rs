//! The app side of the QuickCalc pallet: drawing variables, the clipboard,
//! the command line, picks and the profile settings. The pallet's own state
//! and logic are `ui::window::quick_calc`.

use iced::{Element, Task};

use super::{DocumentTab, Message, OpenCADStudio};
use crate::app::calc::{CalcContext, Value};
use crate::command::CadCommand;
use crate::modules::draw::inquiry::calc_pick::{CalcPickCommand, PickKind};
use crate::ui::dock::PanelId;
use crate::ui::window::quick_calc::{full_text, QuickCalcMsg, VarScope, INPUT_ID};

/// The drawing's parameters that resolve to a number, in definition order.
fn drawing_vars(tab: &DocumentTab) -> Vec<(String, f64)> {
    let table = tab.scene.named_parameters();
    let values = table.resolve_all();
    table
        .iter()
        .filter_map(|p| match values.get(&p.name) {
            Some(Ok(v)) if v.is_finite() => Some((p.name.clone(), *v)),
            _ => None,
        })
        .collect()
}

/// Millimetres per drawing unit; a unitless drawing counts as millimetres.
fn base_unit(tab: &DocumentTab) -> f64 {
    super::properties::insunits_to_mm(tab.scene.document.header.insertion_units).unwrap_or(1.0)
}

/// The drawing unit as the footer and the preview name it.
fn unit_label(tab: &DocumentTab) -> &'static str {
    match tab.scene.document.header.insertion_units {
        1 => "in",
        2 => "ft",
        3 => "mi",
        5 => "cm",
        6 => "m",
        7 => "km",
        10 => "yd",
        14 => "dm",
        13 => "µm",
        _ => "mm",
    }
}

fn focus_input<T: Send + 'static>() -> Task<T> {
    let id = iced::widget::Id::new(INPUT_ID);
    Task::batch([
        iced::widget::operation::focus(id.clone()),
        iced::widget::operation::move_cursor_to_end(id),
    ])
}

fn copy_to_clipboard(text: String) -> Task<Message> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::sys::write_clipboard_text(&text);
        Task::none()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        iced::clipboard::write(text).discard()
    }
}

impl OpenCADStudio {
    pub(in crate::app) fn quick_calc_view<'a>(
        &'a self,
        tab: &'a DocumentTab,
        width: f32,
        chrome: crate::ui::dock::Chrome,
    ) -> Element<'a, Message> {
        let drawing = drawing_vars(tab);
        let pick_step = tab.active_cmd.as_ref().and_then(|c| c.quick_calc_step());
        crate::ui::window::quick_calc::view(
            &self.quick_calc,
            crate::ui::window::quick_calc::Inputs {
                drawing,
                base_unit: base_unit(tab),
                unit_label: unit_label(tab),
                pick_step,
                theme: self.active_theme.clone(),
            },
            width,
            chrome,
        )
    }

    fn quick_calc_context(&self) -> CalcContext {
        let tab = &self.tabs[self.active_tab];
        self.quick_calc.context(&drawing_vars(tab), base_unit(tab))
    }

    /// QUICKCALC / QC: show the pallet (docked on the right the first time)
    /// with the input focused, or close it when it is showing.
    pub(in crate::app) fn toggle_quick_calc(&mut self) -> Task<Message> {
        if self.dock_panel_visible(PanelId::QuickCalc) {
            return self.on_dock(crate::ui::dock::DockMsg::Close(PanelId::QuickCalc));
        }
        self.quick_calc.show = true;
        self.dock_open_at_default(PanelId::QuickCalc);
        self.save_config();
        focus_input()
    }

    fn save_quick_calc_settings(&mut self) {
        self.persist_settings_if_changed();
    }

    /// Escape and Ctrl+Enter reach the app as shortcuts even while the input
    /// has focus. Returns the task when QuickCalc handled the key.
    pub(in crate::app) fn quick_calc_shortcut(&mut self, key: &str) -> Option<Task<Message>> {
        if !self.dock_panel_visible(PanelId::QuickCalc) {
            return None;
        }
        let base = key.rsplit('+').next().unwrap_or(key);
        // An open flyout closes first, wherever the focus is.
        if base == "ESCAPE" && self.quick_calc.flyout.is_some() {
            self.quick_calc.flyout = None;
            return Some(Task::none());
        }
        if !self.quick_calc.typing {
            return None;
        }
        let accel = key.starts_with("CTRL+") || key.starts_with("CMD+");
        if base == "ESCAPE" && self.tabs[self.active_tab].active_cmd.is_none() {
            self.quick_calc.typing = false;
            self.quick_calc.input.clear();
            self.quick_calc.hist_idx = None;
            return Some(Task::none());
        }
        if accel && matches!(base, "ENTER" | "RETURN") {
            return Some(self.on_quick_calc(QuickCalcMsg::PasteToCommandLine));
        }
        None
    }

    pub(in crate::app) fn on_quick_calc(&mut self, msg: QuickCalcMsg) -> Task<Message> {
        let ctx = self.quick_calc_context();
        let qc = &mut self.quick_calc;
        // Doing anything else in the pallet closes an open flyout.
        if !matches!(
            msg,
            QuickCalcMsg::Flyout(_)
                | QuickCalcMsg::CloseFlyout
                | QuickCalcMsg::Key(_)
                | QuickCalcMsg::Memory(_)
                | QuickCalcMsg::MemoryInsert(_)
                | QuickCalcMsg::FlashExpired
                | QuickCalcMsg::HistoryStep(_)
        ) {
            qc.flyout = None;
        }
        match msg {
            QuickCalcMsg::Input(text) => {
                qc.input = text;
                qc.hist_idx = None;
                qc.typing = true;
                Task::none()
            }
            QuickCalcMsg::Submit | QuickCalcMsg::Key(crate::ui::window::quick_calc::Key::Eval) => {
                qc.typing = true;
                if let Some(done) = qc.submit(&ctx) {
                    if let Some(name) = done.assign {
                        self.quick_calc_store(&name, done.value);
                    }
                }
                focus_input()
            }
            QuickCalcMsg::PasteToCommandLine => {
                let Some(value) = qc.current_value(&ctx) else {
                    qc.set_flash(crate::t!("Nothing to paste yet").into_owned());
                    return Task::none();
                };
                let text = full_text(&value);
                qc.set_flash(
                    crate::tf!("Sent {} to the command line", value.display()).into_owned(),
                );
                qc.typing = false;
                self.command_line.input.push_str(&text);
                iced::widget::operation::focus(iced::widget::Id::new(
                    crate::ui::command_line::CMD_INPUT_ID,
                ))
            }
            QuickCalcMsg::Copy => match qc.current_value(&ctx) {
                Some(value) => {
                    qc.set_flash(crate::tf!("Copied {}", value.display()).into_owned());
                    copy_to_clipboard(full_text(&value))
                }
                None => {
                    qc.set_flash(crate::t!("Nothing to copy yet").into_owned());
                    Task::none()
                }
            },
            QuickCalcMsg::HistoryCopy(i) => match qc.history.get(i) {
                Some(entry) => {
                    let value = entry.value;
                    qc.set_flash(crate::tf!("Copied {}", value.display()).into_owned());
                    copy_to_clipboard(full_text(&value))
                }
                None => Task::none(),
            },
            QuickCalcMsg::Pick(kind) => self.quick_calc_pick(kind),
            QuickCalcMsg::CancelPick => {
                let i = self.active_tab;
                if self.tabs[i]
                    .active_cmd
                    .as_ref()
                    .is_some_and(|c| c.quick_calc_step().is_some())
                {
                    return self.update(Message::CommandEscape);
                }
                self.quick_calc.pick = None;
                Task::none()
            }
            QuickCalcMsg::Picked(value) => {
                qc.pick = None;
                qc.insert_value(&value);
                qc.typing = true;
                focus_input()
            }
            QuickCalcMsg::SetMode(mode) => {
                qc.mode = mode;
                self.save_quick_calc_settings();
                Task::none()
            }
            QuickCalcMsg::ClearHistory => {
                qc.history.clear();
                qc.hist_idx = None;
                Task::none()
            }
            QuickCalcMsg::HistoryLoad(i) => {
                if let Some(entry) = qc.history.get(i) {
                    qc.input = entry.expr.clone();
                    qc.hist_idx = Some(i);
                    qc.typing = true;
                }
                focus_input()
            }
            QuickCalcMsg::HistoryStep(up) => {
                qc.history_step(up);
                iced::widget::operation::move_cursor_to_end(iced::widget::Id::new(INPUT_ID))
            }
            QuickCalcMsg::Key(key) => {
                if let Some(status) = qc.press(key) {
                    qc.set_flash(status);
                }
                qc.typing = true;
                focus_input()
            }
            QuickCalcMsg::Memory(op) => {
                if let Some(status) = qc.memory_op(op, &ctx) {
                    qc.set_flash(status);
                }
                qc.typing = true;
                focus_input()
            }
            QuickCalcMsg::MemoryInsert(index) => {
                qc.memory_insert(index);
                qc.typing = true;
                focus_input()
            }
            QuickCalcMsg::SetKeypadMode(mode) => {
                qc.set_keypad_mode(mode);
                self.save_quick_calc_settings();
                Task::none()
            }
            QuickCalcMsg::Flyout(flyout) => {
                qc.toggle_flyout(flyout);
                Task::none()
            }
            QuickCalcMsg::CloseFlyout => {
                qc.flyout = None;
                Task::none()
            }
            QuickCalcMsg::Toggle(section) => {
                qc.toggle(section);
                self.save_quick_calc_settings();
                Task::none()
            }
            QuickCalcMsg::UnitCat(cat) => {
                qc.set_unit_cat(cat);
                Task::none()
            }
            QuickCalcMsg::UnitFrom(unit) => {
                qc.unit_from = unit;
                Task::none()
            }
            QuickCalcMsg::UnitTo(unit) => {
                qc.unit_to = unit;
                Task::none()
            }
            QuickCalcMsg::UnitValue(value) => {
                qc.unit_value = value;
                Task::none()
            }
            QuickCalcMsg::UnitSwap => {
                (qc.unit_from, qc.unit_to) = (qc.unit_to, qc.unit_from);
                Task::none()
            }
            QuickCalcMsg::UnitInsert => {
                if let Some(literal) = qc.converted_literal() {
                    qc.append(&literal);
                    qc.typing = true;
                }
                focus_input()
            }
            QuickCalcMsg::VarInsert(name) => {
                qc.append(&name);
                qc.typing = true;
                focus_input()
            }
            QuickCalcMsg::VarDelete(VarScope::Constant, name) => {
                qc.constants.retain(|(c, _)| *c != name);
                self.save_quick_calc_settings();
                Task::none()
            }
            QuickCalcMsg::VarDelete(VarScope::Drawing, name) => {
                if qc.session.remove(&name).is_none() {
                    let i = self.active_tab;
                    self.delete_parameter(i, &name);
                }
                Task::none()
            }
            QuickCalcMsg::NewVar => {
                qc.input.clear();
                qc.hist_idx = None;
                qc.new_var_hint = true;
                qc.typing = true;
                qc.set_flash(crate::t!("Type name = value, then Enter").into_owned());
                focus_input()
            }
            QuickCalcMsg::FlashExpired => {
                qc.expire_flash();
                Task::none()
            }
        }
    }

    /// Keep `name = value`: in "My constants" when it is one, as a drawing
    /// parameter when it is a number, for the session when it is a point.
    fn quick_calc_store(&mut self, name: &str, value: Value) {
        let was_constant = self.quick_calc.constants.iter().any(|(c, _)| c == name);
        match self.quick_calc.store(name, value) {
            Some(number) => {
                let i = self.active_tab;
                self.apply_parameter_input(i, name, &number.to_string());
            }
            None if was_constant => self.save_quick_calc_settings(),
            None => {}
        }
    }

    /// Start a pick in the drawing. Only one command runs at a time, so a
    /// pick waits until the current command has finished.
    fn quick_calc_pick(&mut self, kind: PickKind) -> Task<Message> {
        let i = self.active_tab;
        if self.tabs[i].active_cmd.is_some() {
            self.quick_calc
                .set_flash(crate::t!("Finish the current command first").into_owned());
            return Task::none();
        }
        self.quick_calc.pick = Some(kind);
        self.quick_calc.typing = false;
        let cmd = CalcPickCommand::new(kind, self.quick_calc.mode);
        self.command_line.push_info(&cmd.prompt());
        self.tabs[i].active_cmd = Some(Box::new(cmd));
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::window::quick_calc::Key;
    use glam::DVec3;

    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        app.tabs[i].scene.document.header.insertion_units = 4;
        app
    }

    fn type_and_submit(app: &mut OpenCADStudio, input: &str) {
        let _ = app.on_quick_calc(QuickCalcMsg::Input(input.into()));
        let _ = app.on_quick_calc(QuickCalcMsg::Submit);
    }

    #[test]
    fn quickcalc_toggles_the_pallet_and_cal_stays_a_command() {
        let mut app = app();
        let _ = app.dispatch_command("QUICKCALC");
        assert!(app.dock_panel_visible(PanelId::QuickCalc));
        let _ = app.dispatch_command("QC");
        assert!(!app.dock_panel_visible(PanelId::QuickCalc));
        let _ = app.dispatch_command("CAL 2+3");
        assert!(app
            .command_line
            .history
            .last()
            .unwrap()
            .text
            .ends_with("= 5"));
        assert!(!app.dock_panel_visible(PanelId::QuickCalc));
    }

    #[test]
    fn assignment_makes_a_drawing_variable() {
        let mut app = app();
        type_and_submit(&mut app, "wall = 3.6m");
        let i = app.active_tab;
        assert_eq!(
            app.tabs[i].scene.named_parameters().resolve("wall").ok(),
            Some(3600.0)
        );
        type_and_submit(&mut app, "wall / 2");
        assert_eq!(app.quick_calc.ans, Some(Value::Number(1800.0)));
        // Updating keeps one parameter.
        type_and_submit(&mut app, "wall = 300");
        assert_eq!(app.tabs[i].scene.named_parameters().len(), 1);
        let _ = app.on_quick_calc(QuickCalcMsg::VarDelete(VarScope::Drawing, "wall".into()));
        assert!(app.tabs[i].scene.named_parameters().is_empty());
    }

    #[test]
    fn constants_are_updated_in_place() {
        let mut app = app();
        app.quick_calc.constants.push(("brick".into(), 210.0));
        type_and_submit(&mut app, "brick = 215");
        assert_eq!(app.quick_calc.constants, vec![("brick".to_string(), 215.0)]);
        let i = app.active_tab;
        assert!(app.tabs[i].scene.named_parameters().is_empty());
    }

    #[test]
    fn pick_runs_as_a_command_and_inserts_its_value() {
        let mut app = app();
        app.quick_calc.show = true;
        let _ = app.on_quick_calc(QuickCalcMsg::Input("wall + ".into()));
        let _ = app.on_quick_calc(QuickCalcMsg::Pick(PickKind::Distance));
        let i = app.active_tab;
        assert_eq!(
            app.tabs[i]
                .active_cmd
                .as_ref()
                .and_then(|c| c.quick_calc_step()),
            Some(1)
        );
        let first = app.tabs[i]
            .active_cmd
            .as_mut()
            .unwrap()
            .on_point(DVec3::ZERO);
        assert!(matches!(first, crate::command::CmdResult::NeedPoint));
        let done = app.tabs[i]
            .active_cmd
            .as_mut()
            .unwrap()
            .on_point(DVec3::new(0.0, 3600.0, 0.0));
        let _ = app.apply_cmd_result(done);
        assert!(app.tabs[i].active_cmd.is_none());
        // The routed message, as the runtime would deliver it.
        let _ = app.on_quick_calc(QuickCalcMsg::Picked(Value::Number(3600.0)));
        assert_eq!(app.quick_calc.input, "wall + 3600");
        assert_eq!(app.quick_calc.pick, None);
    }

    #[test]
    fn pick_waits_for_a_running_command() {
        let mut app = app();
        let _ = app.dispatch_command("LINE");
        let _ = app.on_quick_calc(QuickCalcMsg::Pick(PickKind::Point));
        let i = app.active_tab;
        assert_eq!(app.tabs[i].active_cmd.as_ref().unwrap().name(), "LINE");
        assert!(app.quick_calc.flash.is_some());
    }

    #[test]
    fn paste_sends_the_value_to_the_command_line() {
        let mut app = app();
        let _ = app.on_quick_calc(QuickCalcMsg::Input("[1,2] * 2".into()));
        let _ = app.on_quick_calc(QuickCalcMsg::PasteToCommandLine);
        assert_eq!(app.command_line.input, "2,4");
    }

    #[test]
    fn escape_clears_only_while_typing() {
        let mut app = app();
        app.quick_calc.show = true;
        app.dock_open_at_default(PanelId::QuickCalc);
        let _ = app.on_quick_calc(QuickCalcMsg::Input("12".into()));
        assert!(app.quick_calc_shortcut("ESCAPE").is_some());
        assert!(app.quick_calc.input.is_empty());
        app.quick_calc.input = "12".into();
        app.quick_calc.typing = false;
        assert!(app.quick_calc_shortcut("ESCAPE").is_none());
        assert_eq!(app.quick_calc.input, "12");
    }

    #[test]
    fn keypad_keys_edit_the_input() {
        let mut app = app();
        for k in ["1", "2"] {
            let _ = app.on_quick_calc(QuickCalcMsg::Key(Key::Text(k)));
        }
        let _ = app.on_quick_calc(QuickCalcMsg::Key(Key::Postfix("^2")));
        let _ = app.on_quick_calc(QuickCalcMsg::Key(Key::Eval));
        assert_eq!(app.quick_calc.ans, Some(Value::Number(144.0)));
    }
}

#[cfg(test)]
mod keypad_tests {
    use super::*;
    use crate::ui::window::quick_calc::{Flyout, KeypadMode, MemoryOp, Section};

    #[test]
    fn escape_closes_a_flyout_first() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app.quick_calc.show = true;
        app.dock_open_at_default(PanelId::QuickCalc);
        let _ = app.on_quick_calc(QuickCalcMsg::SetKeypadMode(KeypadMode::Scientific));
        assert!(app.quick_calc.is_open(Section::Keypad));
        let _ = app.on_quick_calc(QuickCalcMsg::Flyout(Flyout::Trigonometry));
        let _ = app.on_quick_calc(QuickCalcMsg::Input("12".into()));
        assert!(app.quick_calc_shortcut("ESCAPE").is_some());
        assert_eq!(app.quick_calc.flyout, None);
        // The next Escape clears the input.
        assert!(app.quick_calc_shortcut("ESCAPE").is_some());
        assert!(app.quick_calc.input.is_empty());
    }

    #[test]
    fn memory_row_and_flyout() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let _ = app.on_quick_calc(QuickCalcMsg::Input("2 * 435".into()));
        let _ = app.on_quick_calc(QuickCalcMsg::Memory(MemoryOp::Store));
        assert_eq!(app.quick_calc.memory, vec![870.0]);
        let _ = app.on_quick_calc(QuickCalcMsg::Input("[1,2]".into()));
        let _ = app.on_quick_calc(QuickCalcMsg::Memory(MemoryOp::Store));
        assert!(app.quick_calc.flash.is_some());
        let _ = app.on_quick_calc(QuickCalcMsg::Input(String::new()));
        let _ = app.on_quick_calc(QuickCalcMsg::Flyout(Flyout::Memory));
        let _ = app.on_quick_calc(QuickCalcMsg::MemoryInsert(0));
        assert_eq!(app.quick_calc.input, "870");
        assert_eq!(app.quick_calc.flyout, None);
    }
}
