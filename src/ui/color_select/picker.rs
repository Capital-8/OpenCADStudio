//! The Select Color panel: state, update logic and view.
//!
//! The panel edits a pending colour and only reports it back on OK, so every
//! caller keeps applying colours through the same `ColorWindowPick` path.

use super::math::{self, Hsb};
use super::widgets::{ColorArea, GradientSlider};
use crate::app::{ArrowKey, ColorPickTarget, Message};
use crate::t;
use crate::ui::properties::acad_color_display;
use codec::types::Color as AcadColor;
use iced::widget::{button, column, container, row, text, text_input, Space};
use iced::{Background, Border, Color, Element, Length, Theme};

const PANEL_WIDTH: f32 = 400.0;
const BODY_HEIGHT: f32 = 352.0;
const CONTROL_HEIGHT: f32 = 28.0;
const SWATCH_WIDTH: f32 = 104.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorPickerTab {
    #[default]
    Index,
    TrueColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrueColorMode {
    #[default]
    Hsb,
    Rgb,
    Gradient,
}

#[derive(Debug, Clone)]
pub enum ColorPickerMsg {
    Tab(ColorPickerTab),
    Mode(TrueColorMode),
    /// A swatch was clicked.
    Pick(AcadColor),
    /// Set the new colour back to the current one.
    Revert,
    Hue(f32),
    Saturation(f32),
    Brightness(f32),
    SaturationBrightness(f32, f32),
    Red(u8),
    Green(u8),
    Blue(u8),
    FieldInput(String),
    FieldSubmit,
    HexInput(String),
    HexSubmit,
    /// Replace the true colour with its nearest index colour.
    UseNearest,
    Arrow(ArrowKey),
    Ok,
    Cancel,
}

/// What the caller has to do after an update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Commit(AcadColor),
    Cancel,
}

pub struct ColorPicker {
    pub target: ColorPickTarget,
    /// The colour shown as "Current". For a mixed selection this is the colour
    /// of the most recently drawn object.
    current: AcadColor,
    varies: bool,
    pending: AcadColor,
    tab: ColorPickerTab,
    mode: TrueColorMode,
    /// Kept separately from `pending` so the hue survives grays and black.
    hsb: Hsb,
    field_draft: Option<String>,
    hex_draft: Option<String>,
}

impl ColorPicker {
    pub fn new(target: ColorPickTarget, current: AcadColor, varies: bool) -> Self {
        let tab = if matches!(current, AcadColor::Rgb { .. }) {
            ColorPickerTab::TrueColor
        } else {
            ColorPickerTab::Index
        };
        let mut picker = Self {
            target,
            current,
            varies,
            pending: current,
            tab,
            mode: TrueColorMode::default(),
            hsb: Hsb {
                h: 0.0,
                s: 0.0,
                b: 1.0,
            },
            field_draft: None,
            hex_draft: None,
        };
        picker.set_pending(current);
        picker
    }

    pub fn pending(&self) -> AcadColor {
        self.pending
    }

    pub fn tab(&self) -> ColorPickerTab {
        self.tab
    }

    pub fn mode(&self) -> TrueColorMode {
        self.mode
    }

    fn set_pending(&mut self, color: AcadColor) {
        self.pending = color;
        self.field_draft = None;
        self.hex_draft = None;
        if let Some((r, g, b)) = math::color_rgb(color) {
            let next = math::rgb_to_hsb(r, g, b);
            self.hsb = Hsb {
                // A gray or black has no hue of its own; keep the last one so
                // the sliders and field don't jump back to red.
                h: if next.s == 0.0 || next.b == 0.0 {
                    self.hsb.h
                } else {
                    next.h
                },
                s: if next.b == 0.0 { self.hsb.s } else { next.s },
                b: next.b,
            };
        }
    }

    fn set_hsb(&mut self, hsb: Hsb) {
        self.hsb = Hsb {
            h: hsb.h.clamp(0.0, 360.0),
            s: hsb.s.clamp(0.0, 1.0),
            b: hsb.b.clamp(0.0, 1.0),
        };
        let (r, g, b) = self.hsb.to_rgb();
        self.pending = AcadColor::Rgb { r, g, b };
        self.field_draft = None;
        self.hex_draft = None;
    }

    fn set_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.set_pending(AcadColor::Rgb { r, g, b });
    }

    /// RGB of the pending colour; logical colours fall back to the sliders.
    fn rgb(&self) -> (u8, u8, u8) {
        math::color_rgb(self.pending).unwrap_or_else(|| self.hsb.to_rgb())
    }

    pub fn update(&mut self, msg: ColorPickerMsg) -> Option<Outcome> {
        match msg {
            ColorPickerMsg::Tab(tab) => self.tab = tab,
            ColorPickerMsg::Mode(mode) => self.mode = mode,
            ColorPickerMsg::Pick(color) => self.set_pending(color),
            ColorPickerMsg::Revert => self.set_pending(self.current),
            ColorPickerMsg::Hue(h) => self.set_hsb(Hsb { h, ..self.hsb }),
            ColorPickerMsg::Saturation(s) => self.set_hsb(Hsb { s, ..self.hsb }),
            ColorPickerMsg::Brightness(b) => self.set_hsb(Hsb { b, ..self.hsb }),
            ColorPickerMsg::SaturationBrightness(s, b) => self.set_hsb(Hsb { s, b, ..self.hsb }),
            ColorPickerMsg::Red(red) => {
                let (_, g, b) = self.rgb();
                self.set_rgb(red, g, b);
            }
            ColorPickerMsg::Green(green) => {
                let (r, _, b) = self.rgb();
                self.set_rgb(r, green, b);
            }
            ColorPickerMsg::Blue(blue) => {
                let (r, g, _) = self.rgb();
                self.set_rgb(r, g, blue);
            }
            ColorPickerMsg::FieldInput(value) => self.field_draft = Some(value),
            ColorPickerMsg::FieldSubmit => {
                if let Some(draft) = self.field_draft.take() {
                    if let Some(color) = math::parse_color_input(&draft) {
                        self.set_pending(color);
                    }
                }
            }
            ColorPickerMsg::HexInput(value) => self.hex_draft = Some(value),
            ColorPickerMsg::HexSubmit => {
                if let Some(draft) = self.hex_draft.take() {
                    if let Some((r, g, b)) = math::parse_hex(&draft) {
                        self.set_rgb(r, g, b);
                    }
                }
            }
            ColorPickerMsg::UseNearest => {
                let (r, g, b) = self.rgb();
                let (index, _) = math::nearest_aci_match(r, g, b);
                self.set_pending(AcadColor::Index(index));
            }
            ColorPickerMsg::Arrow(direction) => {
                if self.tab == ColorPickerTab::Index {
                    if let AcadColor::Index(index) = self.pending {
                        if let Some(next) = step_index(index, direction) {
                            self.set_pending(AcadColor::Index(next));
                        }
                    }
                }
            }
            ColorPickerMsg::Ok => {
                // Apply a value still being typed, as if Enter had been pressed.
                self.update(ColorPickerMsg::FieldSubmit);
                self.update(ColorPickerMsg::HexSubmit);
                return Some(Outcome::Commit(self.pending));
            }
            ColorPickerMsg::Cancel => return Some(Outcome::Cancel),
        }
        None
    }

    pub fn view(&self) -> Element<'_, Message> {
        let body = match self.tab {
            ColorPickerTab::Index => self.index_tab(),
            ColorPickerTab::TrueColor => self.true_color_tab(),
        };

        let (sr, sg, sb) = math::color_rgb(self.pending).unwrap_or((128, 128, 128));
        let info = row![
            container(Space::new())
                .width(12)
                .height(12)
                .style(move |theme: &Theme| swatch_style(theme, rgb_color(sr, sg, sb))),
            text(math::describe(self.pending))
                .size(11)
                .font(iced::Font::MONOSPACE),
        ]
        .spacing(8)
        .align_y(iced::Center);

        let body = container(
            column![
                body,
                Space::new().height(Length::Fill),
                container(Space::new())
                    .width(Length::Fill)
                    .height(1)
                    .style(|theme: &Theme| container::Style {
                        background: Some(Background::Color(theme.palette().background.weak.color)),
                        ..Default::default()
                    }),
                info,
            ]
            .spacing(8),
        )
        .padding(12)
        .width(Length::Fill)
        .height(Length::Fixed(BODY_HEIGHT))
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
                background: Some(Background::Color(palette.background.weakest.color)),
                border: Border {
                    color: palette.background.weak.color,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..Default::default()
            }
        });

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

        column![self.tabs(), self.preview_row(), body, actions]
            .spacing(12)
            .width(Length::Fixed(PANEL_WIDTH))
            .into()
    }

    fn tabs(&self) -> Element<'_, Message> {
        let tab = |label: String, tab: ColorPickerTab| {
            let active = self.tab == tab;
            button(
                container(text(label).size(12))
                    .width(Length::Fill)
                    .align_x(iced::Center),
            )
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
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
                background: Some(Background::Color(palette.background.weakest.color)),
                border: Border {
                    color: palette.background.weak.color,
                    width: 1.0,
                    radius: 7.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
    }

    fn preview_row(&self) -> Element<'_, Message> {
        let caption = |label: String| text(label).size(11).style(muted_text);

        let (current_bg, _) = acad_color_display(self.current);
        let (new_bg, _) = acad_color_display(self.pending);
        let current_label = if self.varies {
            "*VARIES*".to_string()
        } else {
            logical_badge(self.current)
        };
        let swatch_text = |label: String, background: Color| {
            let ink = if ink_is_dark(background) {
                Color::from_rgb8(0x11, 0x12, 0x14)
            } else {
                Color::WHITE
            };
            container(
                text(label)
                    .size(9)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..iced::Font::DEFAULT
                    })
                    .color(ink),
            )
            .center(Length::Fill)
        };
        let current = button(swatch_text(current_label, current_bg))
            .on_press(picker_msg(ColorPickerMsg::Revert))
            .padding(0)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(current_bg)),
                ..Default::default()
            });
        let new = container(swatch_text(logical_badge(self.pending), new_bg))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme: &Theme| container::Style {
                background: Some(Background::Color(new_bg)),
                ..Default::default()
            });
        let divider =
            container(Space::new())
                .width(1)
                .height(Length::Fill)
                .style(|_theme: &Theme| container::Style {
                    background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.35))),
                    ..Default::default()
                });
        let swatch = container(row![current, divider, new])
            .width(SWATCH_WIDTH)
            .height(CONTROL_HEIGHT)
            .padding(1)
            .style(|theme: &Theme| container::Style {
                border: Border {
                    color: theme.palette().background.strong.color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            })
            .clip(true);

        let field_text = self
            .field_draft
            .clone()
            .unwrap_or_else(|| math::color_field_text(self.pending));
        let field = text_input("", &field_text)
            .on_input(|value| picker_msg(ColorPickerMsg::FieldInput(value)))
            .on_submit(picker_msg(ColorPickerMsg::FieldSubmit))
            .size(12)
            .font(iced::Font::MONOSPACE)
            .padding([6, 10])
            .width(Length::Fill);

        row![
            column![
                row![
                    caption(t!("Current").into_owned()),
                    Space::new().width(Length::Fill),
                    caption(t!("New").into_owned()),
                ]
                .width(SWATCH_WIDTH),
                swatch,
            ]
            .spacing(4),
            column![caption(t!("Color").into_owned()), field]
                .spacing(4)
                .width(Length::Fill),
        ]
        .spacing(10)
        .align_y(iced::Bottom)
        .into()
    }

    fn index_tab(&self) -> Element<'_, Message> {
        let section = |label: String| text(label).size(11);

        let mut standard = row![].spacing(5);
        for index in 1u8..=9 {
            standard = standard.push(self.swatch_button(index, 30.0, 26.0));
        }

        // ACI 10–249 come in groups of ten: 24 hue families across, their ten
        // shades down.
        let mut grid = column![].spacing(2);
        for shade in 0u8..10 {
            let mut line = row![].spacing(2);
            for family in 1u8..=24 {
                line = line.push(self.swatch_button(family * 10 + shade, 13.0, 13.0));
            }
            grid = grid.push(line);
        }

        let mut grays = row![].spacing(5);
        for index in 250u8..=255 {
            grays = grays.push(self.swatch_button(index, 30.0, 26.0));
        }

        column![
            column![section(t!("Standard colors").into_owned()), standard].spacing(6),
            column![
                row![
                    section(t!("CAD Color Index (ACI) 10–249").into_owned()),
                    Space::new().width(Length::Fill),
                ],
                grid,
            ]
            .spacing(6),
            column![section(t!("Grayscale 250–255").into_owned()), grays].spacing(6),
        ]
        .spacing(12)
        .into()
    }

    fn swatch_button(&self, index: u8, width: f32, height: f32) -> Element<'_, Message> {
        let (r, g, b) = math::aci_rgb(index).unwrap_or((128, 128, 128));
        let fill = rgb_color(r, g, b);
        let selected = self.pending == AcadColor::Index(index);
        button(Space::new().width(width).height(height))
            .on_press(picker_msg(ColorPickerMsg::Pick(AcadColor::Index(index))))
            .padding(0)
            .style(move |theme: &Theme, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                let accent = theme.palette().primary.base.color;
                button::Style {
                    background: Some(Background::Color(fill)),
                    border: Border {
                        color: if selected || hovered {
                            accent
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
        let mode_button = |content: Element<'static, Message>, mode: TrueColorMode| {
            let active = self.mode == mode;
            button(container(content).center_y(Length::Fill))
                .on_press(picker_msg(ColorPickerMsg::Mode(mode)))
                .height(CONTROL_HEIGHT)
                .padding([0, 10])
                .style(move |theme: &Theme, status| toggle_style(theme, active, status))
        };
        let gradient_icon: Element<'static, Message> = container(Space::new())
            .width(26)
            .height(16)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Gradient(iced::Gradient::Linear(
                    iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_4 * 3.0))
                        .add_stop(0.0, Color::WHITE)
                        .add_stop(0.5, Color::from_rgb8(0x3A, 0xA0, 0xFF))
                        .add_stop(1.0, Color::BLACK),
                ))),
                border: Border {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..Default::default()
            })
            .into();
        let modes = row![
            mode_button(text("HSB").size(12).into(), TrueColorMode::Hsb),
            mode_button(text("RGB").size(12).into(), TrueColorMode::Rgb),
            mode_button(gradient_icon, TrueColorMode::Gradient),
        ]
        .spacing(6);

        let editor = match self.mode {
            TrueColorMode::Hsb => self.hsb_sliders(),
            TrueColorMode::Rgb => self.rgb_sliders(),
            TrueColorMode::Gradient => self.gradient_editor(),
        };

        column![modes, editor, self.hex_row()].spacing(12).into()
    }

    fn hsb_sliders(&self) -> Element<'_, Message> {
        let Hsb { h, s, b } = self.hsb;
        let hue_stops = hue_stops();
        let sat_stops = vec![(0.0, hsb_color(h, 0.0, b)), (1.0, hsb_color(h, 1.0, b))];
        let bright_stops = vec![(0.0, Color::BLACK), (1.0, hsb_color(h, s, 1.0))];
        column![
            slider_row(
                "H",
                GradientSlider::new(h / 360.0, hue_stops, |v| picker_msg(ColorPickerMsg::Hue(
                    v * 360.0
                )))
                .wheel_step(1.0 / 360.0),
                number_box(h.round() as u16, 360, |v| {
                    picker_msg(ColorPickerMsg::Hue(f32::from(v)))
                }),
                "°",
            ),
            slider_row(
                "S",
                GradientSlider::new(s, sat_stops, |v| picker_msg(ColorPickerMsg::Saturation(v))),
                number_box((s * 100.0).round() as u16, 100, |v| {
                    picker_msg(ColorPickerMsg::Saturation(f32::from(v) / 100.0))
                }),
                "%",
            ),
            slider_row(
                "B",
                GradientSlider::new(b, bright_stops, |v| picker_msg(ColorPickerMsg::Brightness(
                    v
                ))),
                number_box((b * 100.0).round() as u16, 100, |v| {
                    picker_msg(ColorPickerMsg::Brightness(f32::from(v) / 100.0))
                }),
                "%",
            ),
        ]
        .spacing(10)
        .into()
    }

    fn rgb_sliders(&self) -> Element<'_, Message> {
        let (r, g, b) = self.rgb();
        let ends = |lo: (u8, u8, u8), hi: (u8, u8, u8)| {
            vec![
                (0.0, rgb_color(lo.0, lo.1, lo.2)),
                (1.0, rgb_color(hi.0, hi.1, hi.2)),
            ]
        };
        let channel = |value: u8| f32::from(value) / 255.0;
        let byte = |v: f32| (v * 255.0).round() as u8;
        column![
            slider_row(
                "R",
                GradientSlider::new(channel(r), ends((0, g, b), (255, g, b)), move |v| {
                    picker_msg(ColorPickerMsg::Red(byte(v)))
                })
                .wheel_step(1.0 / 255.0),
                number_box(u16::from(r), 255, |v| picker_msg(ColorPickerMsg::Red(
                    v as u8
                ))),
                "",
            ),
            slider_row(
                "G",
                GradientSlider::new(channel(g), ends((r, 0, b), (r, 255, b)), move |v| {
                    picker_msg(ColorPickerMsg::Green(byte(v)))
                })
                .wheel_step(1.0 / 255.0),
                number_box(u16::from(g), 255, |v| picker_msg(ColorPickerMsg::Green(
                    v as u8
                ))),
                "",
            ),
            slider_row(
                "B",
                GradientSlider::new(channel(b), ends((r, g, 0), (r, g, 255)), move |v| {
                    picker_msg(ColorPickerMsg::Blue(byte(v)))
                })
                .wheel_step(1.0 / 255.0),
                number_box(u16::from(b), 255, |v| picker_msg(ColorPickerMsg::Blue(
                    v as u8
                ))),
                "",
            ),
        ]
        .spacing(10)
        .into()
    }

    fn gradient_editor(&self) -> Element<'_, Message> {
        let Hsb { h, s, b } = self.hsb;
        let (r, g, bl) = self.rgb();
        let area = ColorArea::new(hsb_color(h, 1.0, 1.0), rgb_color(r, g, bl), s, b, |s, b| {
            picker_msg(ColorPickerMsg::SaturationBrightness(s, b))
        })
        .height(Length::Fixed(118.0));
        let hue = GradientSlider::new(h / 360.0, hue_stops(), |v| {
            picker_msg(ColorPickerMsg::Hue(v * 360.0))
        })
        .vertical()
        .height(Length::Fixed(118.0))
        .wheel_step(1.0 / 360.0);

        let field = |letter: &'static str, input: Element<'static, Message>, unit: &'static str| {
            row![
                text(letter).size(12).font(iced::Font::MONOSPACE),
                input,
                text(unit).size(11).style(muted_text),
            ]
            .spacing(4)
            .align_y(iced::Center)
        };
        let values = row![
            field(
                "H",
                number_box(h.round() as u16, 360, |v| picker_msg(ColorPickerMsg::Hue(
                    f32::from(v)
                ))),
                "°",
            ),
            Space::new().width(Length::Fill),
            field(
                "S",
                number_box((s * 100.0).round() as u16, 100, |v| {
                    picker_msg(ColorPickerMsg::Saturation(f32::from(v) / 100.0))
                }),
                "%",
            ),
            Space::new().width(Length::Fill),
            field(
                "B",
                number_box((b * 100.0).round() as u16, 100, |v| {
                    picker_msg(ColorPickerMsg::Brightness(f32::from(v) / 100.0))
                }),
                "%",
            ),
        ]
        .align_y(iced::Center);

        column![row![area, hue].spacing(12), values]
            .spacing(10)
            .into()
    }

    fn hex_row(&self) -> Element<'_, Message> {
        let (r, g, b) = self.rgb();
        let hex = self
            .hex_draft
            .clone()
            .unwrap_or_else(|| math::hex_string(r, g, b));
        let (nearest, exact) = math::nearest_aci_match(r, g, b);
        let is_index = matches!(self.pending, AcadColor::Index(_));
        let match_label = if exact && !is_index {
            t!("Exact index match")
        } else {
            t!("Nearest index")
        };
        let (nr, ng, nb) = math::aci_rgb(nearest).unwrap_or((128, 128, 128));
        let nearest_fill = rgb_color(nr, ng, nb);
        let highlight = exact && !is_index;
        row![
            text(t!("Hex")).size(11).style(muted_text),
            text_input("", &hex)
                .on_input(|value| picker_msg(ColorPickerMsg::HexInput(value)))
                .on_submit(picker_msg(ColorPickerMsg::HexSubmit))
                .size(12)
                .font(iced::Font::MONOSPACE)
                .padding([5, 8])
                .width(Length::Fixed(92.0)),
            Space::new().width(Length::Fill),
            text(match_label).size(11).style(move |theme: &Theme| {
                let palette = theme.palette();
                text::Style {
                    color: Some(if highlight {
                        palette.success.base.color
                    } else {
                        palette.background.base.text.scale_alpha(0.65)
                    }),
                }
            }),
            button(
                row![
                    container(Space::new())
                        .width(16)
                        .height(16)
                        .style(move |theme: &Theme| swatch_style(theme, nearest_fill)),
                    text(format!("ACI {nearest}"))
                        .size(12)
                        .font(iced::Font::MONOSPACE),
                ]
                .spacing(6)
                .align_y(iced::Center),
            )
            .on_press(picker_msg(ColorPickerMsg::UseNearest))
            .padding([4, 8])
            .style(move |theme: &Theme, status| toggle_style(theme, highlight, status)),
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
    match index {
        1..=9 => match direction {
            ArrowKey::Left if index > 1 => Some(index - 1),
            ArrowKey::Right if index < 9 => Some(index + 1),
            _ => None,
        },
        10..=249 => {
            let family = index / 10;
            let shade = index % 10;
            match direction {
                ArrowKey::Left if family > 1 => Some(index - 10),
                ArrowKey::Right if family < 24 => Some(index + 10),
                ArrowKey::Up if shade > 0 => Some(index - 1),
                ArrowKey::Down if shade < 9 => Some(index + 1),
                _ => None,
            }
        }
        250..=255 => match direction {
            ArrowKey::Left if index > 250 => Some(index - 1),
            ArrowKey::Right if index < 255 => Some(index + 1),
            _ => None,
        },
        _ => None,
    }
}

fn picker_msg(msg: ColorPickerMsg) -> Message {
    Message::ColorPicker(msg)
}

fn slider_row<'a>(
    letter: &'static str,
    slider: GradientSlider<'a, Message>,
    number: Element<'a, Message>,
    unit: &'static str,
) -> Element<'a, Message> {
    row![
        container(text(letter).size(12).font(iced::Font::MONOSPACE))
            .width(12)
            .align_x(iced::Center),
        slider,
        number,
        container(text(unit).size(11).style(muted_text)).width(10),
    ]
    .spacing(8)
    .align_y(iced::Center)
    .into()
}

/// A compact spin box: iced_aw's NumberInput keeps its stacked arrows as long
/// as its padding stays at the default.
fn number_box(
    value: u16,
    max: u16,
    on_change: impl Fn(u16) -> Message + Clone + 'static,
) -> Element<'static, Message> {
    iced_aw::widget::NumberInput::new(&value, 0..=max, on_change)
        .set_size(12.0)
        .font(iced::Font::MONOSPACE)
        .width(Length::Fixed(64.0))
        // Extra right padding keeps the digits clear of the arrows; padding
        // below 5 would make NumberInput swap its arrows for + / − buttons.
        .padding(iced::Padding {
            top: 5.0,
            right: 18.0,
            bottom: 5.0,
            left: 6.0,
        })
        .align_x(iced::alignment::Horizontal::Right)
        .style(
            |theme: &Theme, _status| iced_aw::style::number_input::Style {
                button_background: None,
                icon_color: theme.palette().background.base.text.scale_alpha(0.75),
            },
        )
        .into()
}

fn hue_stops() -> Vec<(f32, Color)> {
    (0..=6)
        .map(|step| {
            let offset = step as f32 / 6.0;
            (offset, hsb_color(offset * 360.0, 1.0, 1.0))
        })
        .collect()
}

fn hsb_color(h: f32, s: f32, b: f32) -> Color {
    let (r, g, b) = math::hsb_to_rgb(h, s, b);
    rgb_color(r, g, b)
}

fn rgb_color(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

fn ink_is_dark(color: Color) -> bool {
    math::ink_is_dark(
        (color.r * 255.0) as u8,
        (color.g * 255.0) as u8,
        (color.b * 255.0) as u8,
    )
}

/// Upper-case label for a logical colour shown on a swatch, empty otherwise.
fn logical_badge(color: AcadColor) -> String {
    match color {
        AcadColor::ByLayer => "BYLAYER".to_string(),
        AcadColor::ByBlock => "BYBLOCK".to_string(),
        AcadColor::None => "NONE".to_string(),
        _ => String::new(),
    }
}

fn muted_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.65)),
    }
}

fn swatch_style(_theme: &Theme, fill: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(fill)),
        border: Border {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
            width: 1.0,
            radius: 3.0.into(),
        },
        ..Default::default()
    }
}

fn segment_style(theme: &Theme, active: bool, status: button::Status) -> button::Style {
    let palette = theme.palette();
    let hovered = matches!(status, button::Status::Hovered);
    button::Style {
        background: if active {
            Some(Background::Color(palette.background.strong.color))
        } else if hovered {
            Some(Background::Color(palette.background.weak.color))
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

fn toggle_style(theme: &Theme, active: bool, status: button::Status) -> button::Style {
    let palette = theme.palette();
    let hovered = matches!(status, button::Status::Hovered);
    button::Style {
        background: Some(Background::Color(if active {
            palette.primary.weak.color
        } else if hovered {
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
    use super::*;

    fn picker(current: AcadColor) -> ColorPicker {
        ColorPicker::new(ColorPickTarget::Ribbon, current, false)
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
    fn ok_applies_text_still_being_typed() {
        let mut p = picker(AcadColor::Index(30));
        p.update(ColorPickerMsg::FieldInput("57,191,252".into()));
        assert_eq!(
            p.update(ColorPickerMsg::Ok),
            Some(Outcome::Commit(AcadColor::Rgb {
                r: 57,
                g: 191,
                b: 252
            }))
        );
    }

    #[test]
    fn invalid_field_text_keeps_the_colour() {
        let mut p = picker(AcadColor::Index(30));
        p.update(ColorPickerMsg::FieldInput("bylayer".into()));
        p.update(ColorPickerMsg::FieldSubmit);
        assert_eq!(p.pending(), AcadColor::Index(30));
    }

    #[test]
    fn sliders_produce_true_colours() {
        let mut p = picker(AcadColor::Index(1));
        p.update(ColorPickerMsg::Hue(120.0));
        assert_eq!(p.pending(), AcadColor::Rgb { r: 0, g: 255, b: 0 });
        p.update(ColorPickerMsg::Red(10));
        assert_eq!(
            p.pending(),
            AcadColor::Rgb {
                r: 10,
                g: 255,
                b: 0
            }
        );
    }

    #[test]
    fn hue_survives_a_trip_through_gray() {
        let mut p = picker(AcadColor::Rgb { r: 0, g: 0, b: 255 });
        p.update(ColorPickerMsg::Saturation(0.0));
        p.update(ColorPickerMsg::Saturation(1.0));
        assert_eq!(p.pending(), AcadColor::Rgb { r: 0, g: 0, b: 255 });
    }

    #[test]
    fn hex_field_sets_a_true_colour() {
        let mut p = picker(AcadColor::Index(1));
        p.update(ColorPickerMsg::HexInput("#3bf".into()));
        p.update(ColorPickerMsg::HexSubmit);
        assert_eq!(
            p.pending(),
            AcadColor::Rgb {
                r: 0x33,
                g: 0xBB,
                b: 0xFF
            }
        );
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
        p.update(ColorPickerMsg::Mode(TrueColorMode::Rgb));
        let mut ui = simulator(&p);
        // The first slider row sits below the tabs, preview row and mode
        // buttons; press near its left end.
        let red_row = ui.find("R").expect("R label");
        let bounds = red_row.visible_bounds().expect("R visible");
        ui.point_at(iced::Point::new(bounds.x + 40.0, bounds.center_y()));
        ui.simulate(iced_test::simulator::click());
        assert!(published(ui)
            .iter()
            .any(|msg| matches!(msg, ColorPickerMsg::Red(v) if *v < 57)));
    }

    #[test]
    fn every_view_renders() {
        let folder = std::env::var_os("OCS_COLOR_PICKER_SNAPSHOTS").map(std::path::PathBuf::from);
        let views: [(&str, AcadColor, ColorPickerTab, TrueColorMode, bool); 6] = [
            (
                "index",
                AcadColor::Index(140),
                ColorPickerTab::Index,
                TrueColorMode::Hsb,
                false,
            ),
            (
                "varies",
                AcadColor::Index(30),
                ColorPickerTab::Index,
                TrueColorMode::Hsb,
                true,
            ),
            (
                "hsb",
                AcadColor::Rgb {
                    r: 57,
                    g: 191,
                    b: 252,
                },
                ColorPickerTab::TrueColor,
                TrueColorMode::Hsb,
                false,
            ),
            (
                "rgb",
                AcadColor::Rgb {
                    r: 57,
                    g: 191,
                    b: 252,
                },
                ColorPickerTab::TrueColor,
                TrueColorMode::Rgb,
                false,
            ),
            (
                "gradient",
                AcadColor::Rgb {
                    r: 57,
                    g: 191,
                    b: 252,
                },
                ColorPickerTab::TrueColor,
                TrueColorMode::Gradient,
                false,
            ),
            (
                "exact",
                AcadColor::Rgb { r: 255, g: 0, b: 0 },
                ColorPickerTab::TrueColor,
                TrueColorMode::Hsb,
                false,
            ),
        ];
        for theme in [
            crate::ui::style::fusion_theme::fusion_black(),
            crate::ui::style::fusion_theme::fusion_white(),
        ] {
            for (name, color, tab, mode, varies) in views {
                let mut p = ColorPicker::new(ColorPickTarget::Properties, color, varies);
                p.update(ColorPickerMsg::Tab(tab));
                p.update(ColorPickerMsg::Mode(mode));
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
