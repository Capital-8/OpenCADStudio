//! The Select Color panel: state, update logic and view.
//!
//! The panel edits a pending colour and only reports it back on OK, so every
//! caller keeps applying colours through the same `ColorWindowPick` path.
//! Opened for an interface colour (an Options background) it shows only the
//! colour editor: no tabs, no index colours.

use super::editor::{rgb_color, ColorEditor, EditorMsg};
use super::eyedropper::{Eyedropper, EyedropperMsg};
use super::math;
use crate::app::{ArrowKey, ColorPickTarget, Message};
use crate::t;
use crate::ui::properties::acad_color_display;
use codec::types::Color as AcadColor;
use iced::widget::{button, column, container, row, text, Space};
use iced::{Background, Border, Color, Element, Length, Theme};

const PANEL_WIDTH: f32 = 400.0;
/// Scope for Tab / Shift+Tab, so focus cycles through the panel's own
/// fields instead of the dialogs and command line underneath.
const PANEL_ID: &str = "select-color-panel";
const BODY_HEIGHT: f32 = 352.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorPickerTab {
    #[default]
    Index,
    TrueColor,
}

#[derive(Debug, Clone)]
pub enum ColorPickerMsg {
    Tab(ColorPickerTab),
    /// A swatch was clicked.
    Pick(AcadColor),
    /// Set the new colour back to the current one.
    Revert,
    Editor(EditorMsg),
    /// Replace the true colour with its nearest index colour.
    UseNearest,
    Arrow(ArrowKey),
    Eyedropper(EyedropperMsg),
    Ok,
    Cancel,
}

/// What the caller has to do after an update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Commit(AcadColor),
    Cancel,
    /// Capture the window for the eyedropper.
    Screenshot,
}

pub struct ColorPicker {
    pub target: ColorPickTarget,
    /// The colour shown as "Current". For a mixed selection this is the colour
    /// of the most recently drawn object.
    current: AcadColor,
    varies: bool,
    pending: AcadColor,
    tab: ColorPickerTab,
    editor: ColorEditor,
    eyedropper: Option<Eyedropper>,
}

impl ColorPicker {
    pub fn new(target: ColorPickTarget, current: AcadColor, varies: bool) -> Self {
        let tab = if matches!(current, AcadColor::Rgb { .. }) {
            ColorPickerTab::TrueColor
        } else {
            ColorPickerTab::Index
        };
        Self {
            target,
            current,
            varies,
            pending: current,
            tab,
            editor: ColorEditor::new(math::color_rgb(current).unwrap_or((255, 255, 255))),
            eyedropper: None,
        }
    }

    pub fn pending(&self) -> AcadColor {
        self.pending
    }

    pub fn tab(&self) -> ColorPickerTab {
        self.tab
    }

    /// Interface colours (Options backgrounds) only get the colour editor.
    fn editor_only(&self) -> bool {
        matches!(self.target, ColorPickTarget::Interface(_))
    }

    fn set_pending(&mut self, color: AcadColor) {
        self.pending = color;
        if let Some(rgb) = math::color_rgb(color) {
            self.editor.set_rgb(rgb);
        }
    }

    fn rgb(&self) -> (u8, u8, u8) {
        self.editor.rgb()
    }

    pub fn update(&mut self, msg: ColorPickerMsg) -> Option<Outcome> {
        // While picking from the screen only the eyedropper listens; Esc
        // backs out of it rather than closing the panel.
        if let Some(eye) = self.eyedropper.as_mut() {
            match msg {
                ColorPickerMsg::Eyedropper(EyedropperMsg::Move(p)) => {
                    return eye.track(p).then_some(Outcome::Screenshot);
                }
                ColorPickerMsg::Eyedropper(EyedropperMsg::Shot(shot)) => eye.set_shot(shot),
                ColorPickerMsg::Eyedropper(EyedropperMsg::Pick) => {
                    if let Some((r, g, b)) = eye.picked() {
                        self.eyedropper = None;
                        self.set_pending(AcadColor::Rgb { r, g, b });
                    }
                }
                ColorPickerMsg::Eyedropper(EyedropperMsg::Cancel) | ColorPickerMsg::Cancel => {
                    self.eyedropper = None;
                }
                _ => {}
            }
            return None;
        }
        match msg {
            ColorPickerMsg::Tab(tab) => self.tab = tab,
            ColorPickerMsg::Pick(color) => self.set_pending(color),
            ColorPickerMsg::Revert => self.set_pending(self.current),
            ColorPickerMsg::Editor(editor) => {
                if self.editor.update(editor) {
                    let (r, g, b) = self.editor.rgb();
                    self.pending = AcadColor::Rgb { r, g, b };
                }
            }
            ColorPickerMsg::UseNearest => {
                let (r, g, b) = self.rgb();
                self.set_pending(AcadColor::Index(math::nearest_aci(r, g, b)));
            }
            ColorPickerMsg::Arrow(direction) => {
                if let (ColorPickerTab::Index, AcadColor::Index(index)) = (self.tab, self.pending) {
                    if let Some(next) = step_index(index, direction) {
                        self.set_pending(AcadColor::Index(next));
                    }
                }
            }
            ColorPickerMsg::Eyedropper(EyedropperMsg::Start) => {
                self.eyedropper = Some(Eyedropper::default());
            }
            ColorPickerMsg::Eyedropper(_) => {}
            ColorPickerMsg::Ok => {
                // Apply a value still being typed, as if Enter had been pressed.
                self.update(ColorPickerMsg::Editor(EditorMsg::HexSubmit));
                return Some(Outcome::Commit(self.pending));
            }
            ColorPickerMsg::Cancel => return Some(Outcome::Cancel),
        }
        None
    }

    /// The full-window eyedropper layer, while it is active.
    pub fn eyedropper_view(&self) -> Option<Element<'_, Message>> {
        self.eyedropper.as_ref().map(Eyedropper::view)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let editor_only = self.editor_only();
        let body = if editor_only || self.tab == ColorPickerTab::TrueColor {
            self.true_color_tab()
        } else {
            self.index_tab()
        };

        // The interface picker shrinks to the editor; the CAD panel keeps one
        // height across its tabs, with the info line at the bottom.
        let mut body = column![body].spacing(8);
        if !editor_only {
            body = body.push(Space::new().height(Length::Fill));
            let fill =
                math::color_rgb(self.pending).map_or(Color::from_rgb8(128, 128, 128), rgb_color);
            body = body
                .push(
                    container(Space::new())
                        .width(Length::Fill)
                        .height(1)
                        .style(|theme: &Theme| filled(theme.palette().background.weak.color)),
                )
                .push(
                    row![
                        container(Space::new())
                            .width(12)
                            .height(12)
                            .style(move |_theme: &Theme| swatch_style(fill)),
                        text(math::describe(self.pending))
                            .size(11)
                            .font(iced::Font::MONOSPACE),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                );
        }
        let body = container(body)
            .padding(12)
            .width(Length::Fill)
            .height(if editor_only {
                Length::Shrink
            } else {
                Length::Fixed(BODY_HEIGHT)
            })
            .style(panel_style);

        let actions = row![
            Space::new().width(Length::Fill),
            button(text(t!("Cancel")).size(12))
                .on_press(picker_msg(ColorPickerMsg::Cancel))
                .style(button::secondary)
                .padding([6, 16]),
            button(text(t!("OK")).size(12))
                .on_press(picker_msg(ColorPickerMsg::Ok))
                .style(button::primary)
                .padding([6, 22]),
        ]
        .spacing(8);

        let mut panel = column![].spacing(12).width(PANEL_WIDTH);
        if !editor_only {
            panel = panel.push(self.tabs());
        }
        container(panel.push(self.preview_row()).push(body).push(actions))
            .id(PANEL_ID)
            .into()
    }

    fn tabs(&self) -> Element<'_, Message> {
        let tab = |label: String, tab: ColorPickerTab| {
            let active = self.tab == tab;
            button(container(text(label).size(12)).center_x(Length::Fill))
                .on_press(picker_msg(ColorPickerMsg::Tab(tab)))
                .width(Length::Fill)
                .padding([5, 4])
                .style(move |theme: &Theme, status| segment_style(theme, active, status))
        };
        container(
            row![
                tab(t!("Index Color").into_owned(), ColorPickerTab::Index),
                tab(t!("True Color").into_owned(), ColorPickerTab::TrueColor),
            ]
            .spacing(2),
        )
        .padding(2)
        .style(panel_style)
        .into()
    }

    /// Current and New side by side across the full width, the divider
    /// under the middle of the tab bar. Clicking Current reverts.
    fn preview_row(&self) -> Element<'_, Message> {
        let half = |label: String, value: String, fill: Color| {
            let ink = if ink_is_dark(fill) {
                Color::from_rgb8(0x11, 0x12, 0x14)
            } else {
                Color::WHITE
            };
            column![
                text(label).size(11).color(ink.scale_alpha(0.85)),
                Space::new().height(Length::Fill),
                text(value).size(11).font(iced::Font::MONOSPACE).color(ink),
            ]
            .padding([8, 10])
            .width(Length::Fill)
            .height(Length::Fill)
        };
        let (current_bg, _) = acad_color_display(self.current);
        let (new_bg, _) = acad_color_display(self.pending);
        let current_value = if self.varies {
            "*VARIES*".to_string()
        } else {
            math::short_name(self.current)
        };
        let current = button(half(t!("Current").into_owned(), current_value, current_bg))
            .on_press(picker_msg(ColorPickerMsg::Revert))
            .padding(0)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme: &Theme, _status| button::Style {
                background: Some(current_bg.into()),
                border: Border {
                    radius: iced::border::radius(7.0).right(0.0),
                    ..Border::default()
                },
                ..Default::default()
            });
        let divider = container(Space::new())
            .width(1)
            .height(Length::Fill)
            .style(|_theme: &Theme| filled(Color::from_rgba(0.0, 0.0, 0.0, 0.35)));
        let new = container(half(
            t!("New").into_owned(),
            math::short_name(self.pending),
            new_bg,
        ))
        .style(move |_theme: &Theme| container::Style {
            border: Border {
                radius: iced::border::radius(7.0).left(0.0),
                ..Border::default()
            },
            ..filled(new_bg)
        });
        container(row![current, divider, new])
            .width(Length::Fill)
            .height(60)
            .padding(1)
            .style(|theme: &Theme| container::Style {
                border: Border {
                    color: theme.palette().background.strong.color,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..Default::default()
            })
            .clip(true)
            .into()
    }

    fn index_tab(&self) -> Element<'_, Message> {
        let section = |label: String| text(label).size(11);
        let swatches = |range: std::ops::RangeInclusive<u8>| {
            range.fold(row![].spacing(5), |line, index| {
                line.push(self.swatch_button(index, 30.0, 26.0))
            })
        };
        // ACI 10–249 come in groups of ten: 24 hue families across, their ten
        // shades down.
        let grid = (0u8..10).fold(column![].spacing(2), |grid, shade| {
            grid.push((1u8..=24).fold(row![].spacing(2), |line, family| {
                line.push(self.swatch_button(family * 10 + shade, 13.0, 13.0))
            }))
        });
        column![
            column![section(t!("Standard colors").into_owned()), swatches(1..=9)].spacing(6),
            column![
                section(t!("CAD Color Index (ACI) 10–249").into_owned()),
                grid
            ]
            .spacing(6),
            column![
                section(t!("Grayscale 250–255").into_owned()),
                swatches(250..=255)
            ]
            .spacing(6),
        ]
        .spacing(12)
        .into()
    }

    fn swatch_button(&self, index: u8, width: f32, height: f32) -> Element<'_, Message> {
        let fill = rgb_color(math::aci_rgb(index).unwrap_or((128, 128, 128)));
        let selected = self.pending == AcadColor::Index(index);
        button(Space::new().width(width).height(height))
            .on_press(picker_msg(ColorPickerMsg::Pick(AcadColor::Index(index))))
            .padding(0)
            .style(move |theme: &Theme, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(fill.into()),
                    border: Border {
                        color: if selected || hovered {
                            theme.palette().primary.base.color
                        } else {
                            Color::from_rgba(0.0, 0.0, 0.0, 0.35)
                        },
                        width: if selected {
                            2.5
                        } else if hovered {
                            1.5
                        } else {
                            1.0
                        },
                        radius: 2.0.into(),
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    fn true_color_tab(&self) -> Element<'_, Message> {
        // The eyedropper reads the window back from the GPU, which the web
        // build cannot do.
        let pipette: Option<Element<'_, Message>> = if cfg!(target_arch = "wasm32") {
            None
        } else {
            Some(
                // Just the icon, without a button frame.
                button(crate::ui::icons::themed(crate::ui::icons::EYEDROPPER, 24.0))
                    .on_press(picker_msg(ColorPickerMsg::Eyedropper(EyedropperMsg::Start)))
                    .padding(0)
                    .style(|_theme: &Theme, _status| button::Style::default())
                    .into(),
            )
        };
        let nearest = (!self.editor_only()).then(|| self.nearest_button());
        self.editor.view(pipette, nearest)
    }

    fn nearest_button(&self) -> Element<'_, Message> {
        let (r, g, b) = self.rgb();
        let nearest = math::nearest_aci(r, g, b);
        let nearest_rgb = math::aci_rgb(nearest).unwrap_or((128, 128, 128));
        let exact = nearest_rgb == (r, g, b) && !matches!(self.pending, AcadColor::Index(_));
        let fill = rgb_color(nearest_rgb);
        row![
            text(if exact {
                t!("Exact index match")
            } else {
                t!("Nearest index")
            })
            .size(11)
            .style(move |theme: &Theme| text::Style {
                color: Some(if exact {
                    theme.palette().success.base.color
                } else {
                    theme.palette().background.base.text.scale_alpha(0.65)
                }),
            }),
            button(
                row![
                    container(Space::new())
                        .width(16)
                        .height(16)
                        .style(move |_theme: &Theme| swatch_style(fill)),
                    text(format!("ACI {nearest}"))
                        .size(12)
                        .font(iced::Font::MONOSPACE),
                ]
                .spacing(6)
                .align_y(iced::Center),
            )
            .on_press(picker_msg(ColorPickerMsg::UseNearest))
            .padding([4, 8])
            .style(move |theme: &Theme, status| toggle_style(theme, exact, status)),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    }
}

/// Move through the index palette with the arrow keys: the 24×10 block moves
/// by family (left/right) and shade (up/down); the standard and gray rows
/// move along their row.
fn step_index(index: u8, direction: ArrowKey) -> Option<u8> {
    let along_row = |first: u8, last: u8| match direction {
        ArrowKey::Left if index > first => Some(index - 1),
        ArrowKey::Right if index < last => Some(index + 1),
        _ => None,
    };
    match index {
        1..=9 => along_row(1, 9),
        250..=255 => along_row(250, 255),
        10..=249 => {
            let (family, shade) = (index / 10, index % 10);
            match direction {
                ArrowKey::Left if family > 1 => Some(index - 10),
                ArrowKey::Right if family < 24 => Some(index + 10),
                ArrowKey::Up if shade > 0 => Some(index - 1),
                ArrowKey::Down if shade < 9 => Some(index + 1),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Move keyboard focus to the next (or previous) field inside the panel.
pub fn focus_step<T>(forward: bool) -> iced::Task<T> {
    use iced::advanced::widget::operation::{focusable, scope};
    let id = iced::widget::Id::new(PANEL_ID);
    iced_runtime::task::effect(if forward {
        iced_runtime::Action::widget(scope(id, focusable::focus_next()))
    } else {
        iced_runtime::Action::widget(scope(id, focusable::focus_previous()))
    })
}

fn picker_msg(msg: ColorPickerMsg) -> Message {
    Message::ColorPicker(msg)
}

fn ink_is_dark(color: Color) -> bool {
    let byte = |c: f32| (c * 255.0).round() as u8;
    math::ink_is_dark(byte(color.r), byte(color.g), byte(color.b))
}

fn filled(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        ..Default::default()
    }
}

pub(super) fn muted_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.65)),
    }
}

fn swatch_style(fill: Color) -> container::Style {
    container::Style {
        background: Some(fill.into()),
        border: Border {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
            width: 1.0,
            radius: 3.0.into(),
        },
        ..Default::default()
    }
}

fn panel_style(theme: &Theme) -> container::Style {
    let palette = theme.palette();
    container::Style {
        background: Some(palette.background.weakest.color.into()),
        border: Border {
            color: palette.background.weak.color,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

fn segment_style(theme: &Theme, active: bool, status: button::Status) -> button::Style {
    let palette = theme.palette();
    button::Style {
        background: if active {
            Some(palette.background.strong.color.into())
        } else if matches!(status, button::Status::Hovered) {
            Some(palette.background.weak.color.into())
        } else {
            None
        },
        text_color: if active {
            palette.background.strong.text
        } else {
            palette.background.base.text
        },
        border: Border {
            radius: 5.0.into(),
            ..Border::default()
        },
        ..Default::default()
    }
}

pub(super) fn toggle_style(theme: &Theme, active: bool, status: button::Status) -> button::Style {
    let palette = theme.palette();
    button::Style {
        background: Some(Background::Color(if active {
            palette.primary.weak.color
        } else if matches!(status, button::Status::Hovered) {
            palette.background.weak.color
        } else {
            palette.background.base.color
        })),
        text_color: if active {
            palette.primary.weak.text
        } else {
            palette.background.base.text
        },
        border: Border {
            color: if active {
                palette.primary.base.color
            } else {
                palette.background.strong.color
            },
            width: 1.0,
            radius: 5.0.into(),
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::editor::EditorMode;
    use super::*;

    fn picker(current: AcadColor) -> ColorPicker {
        ColorPicker::new(ColorPickTarget::Ribbon, current, false)
    }

    fn edit(p: &mut ColorPicker, msg: EditorMsg) {
        p.update(ColorPickerMsg::Editor(msg));
    }

    #[test]
    fn opens_on_the_tab_that_fits_the_current_colour() {
        assert_eq!(picker(AcadColor::Index(30)).tab(), ColorPickerTab::Index);
        assert_eq!(
            picker(AcadColor::Rgb { r: 1, g: 2, b: 3 }).tab(),
            ColorPickerTab::TrueColor
        );
        assert_eq!(picker(AcadColor::ByLayer).tab(), ColorPickerTab::Index);
    }

    #[test]
    fn revert_restores_the_current_colour() {
        let mut p = picker(AcadColor::Index(30));
        p.update(ColorPickerMsg::Pick(AcadColor::Index(140)));
        assert_eq!(p.pending(), AcadColor::Index(140));
        p.update(ColorPickerMsg::Revert);
        assert_eq!(p.pending(), AcadColor::Index(30));
    }

    #[test]
    fn ok_commits_and_cancel_does_not() {
        let mut p = picker(AcadColor::Index(30));
        p.update(ColorPickerMsg::Pick(AcadColor::Index(5)));
        assert_eq!(
            p.update(ColorPickerMsg::Ok),
            Some(Outcome::Commit(AcadColor::Index(5)))
        );
        assert_eq!(p.update(ColorPickerMsg::Cancel), Some(Outcome::Cancel));
    }

    #[test]
    fn ok_applies_a_hex_value_still_being_typed() {
        let mut p = picker(AcadColor::Index(30));
        edit(&mut p, EditorMsg::HexInput("#3bf".into()));
        assert_eq!(
            p.update(ColorPickerMsg::Ok),
            Some(Outcome::Commit(AcadColor::Rgb {
                r: 0x33,
                g: 0xBB,
                b: 0xFF
            }))
        );
    }

    #[test]
    fn the_editor_produces_true_colours() {
        let mut p = picker(AcadColor::Index(1));
        edit(&mut p, EditorMsg::Hue(120.0));
        assert_eq!(p.pending(), AcadColor::Rgb { r: 0, g: 255, b: 0 });
        edit(&mut p, EditorMsg::Mode(EditorMode::Rgb));
        assert_eq!(p.pending(), AcadColor::Rgb { r: 0, g: 255, b: 0 });
    }

    #[test]
    fn nearest_turns_a_true_colour_into_an_index() {
        let mut p = picker(AcadColor::Rgb { r: 250, g: 4, b: 4 });
        p.update(ColorPickerMsg::UseNearest);
        assert_eq!(p.pending(), AcadColor::Index(1));
    }

    #[test]
    fn arrow_keys_walk_the_index_grid() {
        assert_eq!(step_index(140, ArrowKey::Right), Some(150));
        assert_eq!(step_index(140, ArrowKey::Left), Some(130));
        assert_eq!(step_index(140, ArrowKey::Down), Some(141));
        assert_eq!(step_index(140, ArrowKey::Up), None);
        assert_eq!(step_index(249, ArrowKey::Right), None);
        assert_eq!(step_index(5, ArrowKey::Right), Some(6));
        assert_eq!(step_index(255, ArrowKey::Right), None);

        let mut p = picker(AcadColor::Index(140));
        p.update(ColorPickerMsg::Arrow(ArrowKey::Right));
        assert_eq!(p.pending(), AcadColor::Index(150));
    }

    #[test]
    fn eyedropper_picks_a_pixel_and_esc_only_leaves_it() {
        let mut p = picker(AcadColor::Index(30));
        p.update(ColorPickerMsg::Eyedropper(EyedropperMsg::Start));
        assert_eq!(
            p.update(ColorPickerMsg::Eyedropper(EyedropperMsg::Move(
                iced::Point::ORIGIN
            ))),
            Some(Outcome::Screenshot)
        );
        // Esc and Enter while picking never close the panel.
        assert_eq!(p.update(ColorPickerMsg::Ok), None);
        assert_eq!(p.update(ColorPickerMsg::Cancel), None);
        assert!(p.eyedropper_view().is_none());

        p.update(ColorPickerMsg::Eyedropper(EyedropperMsg::Start));
        p.update(ColorPickerMsg::Eyedropper(EyedropperMsg::Move(
            iced::Point::ORIGIN,
        )));
        let shot = iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.0);
        p.update(ColorPickerMsg::Eyedropper(EyedropperMsg::Shot(shot)));
        p.update(ColorPickerMsg::Eyedropper(EyedropperMsg::Pick));
        assert!(p.eyedropper_view().is_none());
        assert_eq!(
            p.pending(),
            AcadColor::Rgb {
                r: 10,
                g: 20,
                b: 30
            }
        );
    }

    // ── Headless UI tests ───────────────────────────────────────────────
    //
    // These run the real widgets through iced's simulator. Setting
    // OCS_COLOR_PICKER_SNAPSHOTS to a folder also writes a PNG per view.

    fn simulator(picker: &ColorPicker) -> iced_test::simulator::Simulator<'_, Message> {
        iced_test::simulator::Simulator::with_size(
            iced::Settings {
                fonts: vec![std::borrow::Cow::Borrowed(iced_aw::ICED_AW_FONT_BYTES)],
                ..iced::Settings::default()
            },
            (PANEL_WIDTH + 24.0, 560.0),
            container(picker.view()).padding(12),
        )
    }

    fn published(ui: iced_test::simulator::Simulator<'_, Message>) -> Vec<ColorPickerMsg> {
        ui.into_messages()
            .filter_map(|message| match message {
                Message::ColorPicker(msg) => Some(msg),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn clicking_a_tab_switches_it() {
        let p = picker(AcadColor::Index(30));
        let mut ui = simulator(&p);
        ui.click(t!("True Color").as_ref()).expect("True Color tab");
        assert!(published(ui)
            .iter()
            .any(|msg| matches!(msg, ColorPickerMsg::Tab(ColorPickerTab::TrueColor))));
    }

    #[test]
    fn dragging_a_slider_publishes_values() {
        let mut p = picker(AcadColor::Rgb {
            r: 57,
            g: 191,
            b: 252,
        });
        edit(&mut p, EditorMsg::Mode(EditorMode::Rgb));
        let mut ui = simulator(&p);
        let label = ui.find("R").expect("R label");
        let bounds = label.visible_bounds().expect("R visible");
        ui.point_at(iced::Point::new(bounds.x + 40.0, bounds.center_y()));
        ui.simulate(iced_test::simulator::click());
        assert!(published(ui)
            .iter()
            .any(|msg| matches!(msg, ColorPickerMsg::Editor(EditorMsg::Red(v)) if *v < 57)));
    }

    #[test]
    fn interface_colours_show_only_the_editor() {
        let p = ColorPicker::new(
            ColorPickTarget::Interface(crate::app::UiColor::ModelBackground),
            AcadColor::Rgb {
                r: 30,
                g: 30,
                b: 30,
            },
            false,
        );
        let mut ui = simulator(&p);
        assert!(ui.find(t!("Index Color").as_ref()).is_err());
        assert!(ui.find(t!("Nearest index").as_ref()).is_err());
        assert!(ui.find("HSB").is_ok());
    }

    #[test]
    fn every_view_renders() {
        let folder = std::env::var_os("OCS_COLOR_PICKER_SNAPSHOTS").map(std::path::PathBuf::from);
        let blue = AcadColor::Rgb {
            r: 57,
            g: 191,
            b: 252,
        };
        let views: [(
            &str,
            ColorPickTarget,
            AcadColor,
            ColorPickerTab,
            EditorMode,
            bool,
        ); 7] = [
            (
                "index",
                ColorPickTarget::Properties,
                AcadColor::Index(140),
                ColorPickerTab::Index,
                EditorMode::Hsb,
                false,
            ),
            (
                "varies",
                ColorPickTarget::Properties,
                AcadColor::Index(30),
                ColorPickerTab::Index,
                EditorMode::Hsb,
                true,
            ),
            (
                "hsb",
                ColorPickTarget::Properties,
                blue,
                ColorPickerTab::TrueColor,
                EditorMode::Hsb,
                false,
            ),
            (
                "rgb",
                ColorPickTarget::Properties,
                blue,
                ColorPickerTab::TrueColor,
                EditorMode::Rgb,
                false,
            ),
            (
                "field",
                ColorPickTarget::Properties,
                blue,
                ColorPickerTab::TrueColor,
                EditorMode::Field,
                false,
            ),
            (
                "exact",
                ColorPickTarget::Properties,
                AcadColor::Rgb { r: 255, g: 0, b: 0 },
                ColorPickerTab::TrueColor,
                EditorMode::Hsb,
                false,
            ),
            (
                "interface",
                ColorPickTarget::Interface(crate::app::UiColor::ModelBackground),
                AcadColor::Rgb {
                    r: 33,
                    g: 40,
                    b: 48,
                },
                ColorPickerTab::TrueColor,
                EditorMode::Hsb,
                false,
            ),
        ];
        for theme in [
            crate::ui::style::fusion_theme::fusion_black(),
            crate::ui::style::fusion_theme::fusion_white(),
        ] {
            for (name, target, color, tab, mode, varies) in views.clone() {
                let mut p = ColorPicker::new(target, color, varies);
                p.update(ColorPickerMsg::Tab(tab));
                edit(&mut p, EditorMsg::Mode(mode));
                let mut ui = simulator(&p);
                let snapshot = ui.snapshot(&theme).expect("snapshot");
                if let Some(folder) = &folder {
                    let shade = if theme.palette().background.base.color.r < 0.5 {
                        "dark"
                    } else {
                        "light"
                    };
                    let path = folder.join(format!("{shade}-{name}.png"));
                    let _ = std::fs::remove_file(&path);
                    snapshot.matches_image(&path).expect("write snapshot");
                }
            }
        }
    }
}
