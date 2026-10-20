//! The QuickCalc pallet's layout: toolbar, history, input with its preview
//! or error, the collapsible sections and the footer.

use iced::widget::text::{Ellipsis, Span, Wrapping};
use iced::widget::{
    button, column, container, rich_text, row, scrollable, svg, text, text_input, tooltip, Space,
};
use iced::{Background, Border, Color, Element, Fill, Font, Length, Theme};

use super::{
    assignment_target, continues_ans, has_length_suffix, Key, Preview, QuickCalcMsg,
    QuickCalcPalette, Section, UnitCat, VarScope, INPUT_ID,
};
use crate::app::calc::{AngleMode, Value};
use crate::app::Message;
use crate::modules::draw::inquiry::calc_pick::PickKind;
use crate::ui::dock::{Chrome, PanelId};

pub const ICON: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/quickcalc.svg");
const POINT: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/point.svg");
const DISTANCE: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/distance.svg");
const ANGLE: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/angle.svg");
const COPY: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/copy.svg");
const SEND: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/send.svg");
const CHEVRON_DOWN: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/chevron_down.svg");
const CHEVRON_RIGHT: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/chevron_right.svg");
const SWAP: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/swap.svg");
const DELETE: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/delete.svg");
const PLUS: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/plus.svg");
const ERROR: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/error.svg");

const MONO: Font = Font::MONOSPACE;
/// The heavier weight for keys, section headers and results. The default
/// font is the system's, which often has no Medium, and asking for one
/// would quietly come back Regular; Semibold is reliably there.
const STRONG: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..Font::DEFAULT
};
const MONO_STRONG: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..Font::MONOSPACE
};

fn m(msg: QuickCalcMsg) -> Message {
    Message::QuickCalc(msg)
}

// ── Colours ─────────────────────────────────────────────────────────────

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

fn luminance(c: Color) -> f32 {
    let channel = |v: f32| {
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Text for the accent fill: near-black (#06121A) or white, whichever reads
/// better, so "=" keeps at least 4.5:1 on any theme's accent.
fn on_accent(accent: Color) -> Color {
    let dark = Color::from_rgb8(0x06, 0x12, 0x1A);
    if contrast(dark, accent) >= contrast(Color::WHITE, accent) {
        dark
    } else {
        Color::WHITE
    }
}

/// The pallet's colours, all derived from the theme so Fusion Black, Fusion
/// White and every other theme get a matching pallet.
#[derive(Clone, Copy)]
struct Tokens {
    weak: Color,
    strong: Color,
    line: Color,
    text: Color,
    muted: Color,
    faint: Color,
    accent: Color,
    on_accent: Color,
    warn: Color,
    danger: Color,
    /// Error text: the danger colour, lightened on dark chrome for contrast.
    danger_text: Color,
    field: Color,
    digit: Color,
    digit_line: Color,
}

fn tokens(theme: &Theme) -> Tokens {
    let p = theme.palette();
    let dark = p.is_dark;
    let bg = p.background.base.color;
    let text = p.background.base.text;
    let line = p.background.neutral.color;
    Tokens {
        weak: p.background.weak.color,
        strong: p.background.strong.color,
        line,
        text,
        muted: mix(bg, text, 0.68),
        faint: mix(bg, text, 0.52),
        accent: p.primary.base.color,
        on_accent: on_accent(p.primary.base.color),
        warn: p.warning.base.color,
        danger: p.danger.base.color,
        danger_text: if dark {
            Color::from_rgb8(0xFF, 0x6B, 0x66)
        } else {
            p.danger.base.color
        },
        field: if dark {
            mix(bg, Color::BLACK, 0.3)
        } else {
            Color::WHITE
        },
        digit: if dark {
            mix(bg, Color::WHITE, 0.2)
        } else {
            Color::WHITE
        },
        digit_line: if dark {
            mix(bg, Color::WHITE, 0.28)
        } else {
            mix(line, Color::BLACK, 0.12)
        },
    }
}

fn icon<'a>(bytes: &'static [u8], size: f32, color: fn(&Tokens) -> Color) -> Element<'a, Message> {
    svg(crate::ui::icons::themed_handle(bytes))
        .width(size)
        .height(size)
        .style(move |theme: &Theme, _| svg::Style {
            color: Some(color(&tokens(theme))),
        })
        .into()
}

/// A tooltip as the ribbon shows them: after a short delay, on a panel.
fn tip<'a>(content: impl Into<Element<'a, Message>>, label: String) -> Element<'a, Message> {
    tooltip(
        content,
        crate::ui::ribbon::tooltip_content(label),
        tooltip::Position::Bottom,
    )
    .delay(std::time::Duration::from_millis(400))
    .style(crate::ui::ribbon::tooltip_style)
    .into()
}

/// Full-strength text in the theme's text colour.
fn strong<'a>(s: impl text::IntoFragment<'a>, size: f32, font: Font) -> text::Text<'a, Theme> {
    text(s)
        .size(size)
        .font(font)
        .style(|theme: &Theme| text::Style {
            color: Some(tokens(theme).text),
        })
}

fn mono<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a, Theme> {
    text(s).size(size).font(MONO)
}

fn muted<'a>(s: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a, Theme> {
    text(s).size(size).style(|theme: &Theme| text::Style {
        color: Some(tokens(theme).muted),
    })
}

// ── Buttons ─────────────────────────────────────────────────────────────

/// A 28 px toolbar button; `on` highlights the running pick tool.
fn tool<'a>(
    bytes: &'static [u8],
    label: String,
    msg: QuickCalcMsg,
    on: bool,
) -> Element<'a, Message> {
    let b = button(
        container(icon(
            bytes,
            18.0,
            if on { |t| t.accent } else { |t| t.muted },
        ))
        .center(Fill),
    )
    .width(28)
    .height(28)
    .padding(0)
    .on_press(m(msg))
    .style(move |theme: &Theme, status| {
        let t = tokens(theme);
        let background = if on {
            Some(Background::Color(t.accent.scale_alpha(0.16)))
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(Background::Color(t.strong))
        } else {
            None
        };
        button::Style {
            background,
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            text_color: t.text,
            ..Default::default()
        }
    });
    tip(b, label)
}

fn plain_button_style(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then_some(Background::Color(t.strong)),
        border: Border {
            color: t.line,
            width: 1.0,
            radius: 3.0.into(),
        },
        text_color: t.text,
        ..Default::default()
    }
}

/// A bordered text button (Cancel, Insert, New variable).
fn text_button<'a>(
    content: impl Into<Element<'a, Message>>,
    msg: QuickCalcMsg,
) -> Element<'a, Message> {
    button(content)
        .padding([4, 10])
        .on_press(m(msg))
        .style(plain_button_style)
        .into()
}

/// One option of a segmented control.
fn segment<'a>(label: String, on: bool, msg: QuickCalcMsg, fill: bool) -> Element<'a, Message> {
    let label = text(label).size(10.5);
    let content = if fill {
        container(label).center(Fill)
    } else {
        container(label).center_y(Fill)
    };
    let b = button(content)
        .height(20)
        .padding([0, 8])
        .on_press(m(msg))
        .style(move |theme: &Theme, status| {
            let t = tokens(theme);
            button::Style {
                background: on.then_some(Background::Color(t.strong)),
                border: Border {
                    radius: 2.0.into(),
                    ..Default::default()
                },
                text_color: if on || matches!(status, button::Status::Hovered) {
                    t.text
                } else {
                    t.muted
                },
                ..Default::default()
            }
        });
    if fill {
        b.width(Fill).into()
    } else {
        b.into()
    }
}

fn segmented<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    container(row(items).spacing(1))
        .padding(2)
        .style(|theme: &Theme| container::Style {
            border: Border {
                color: tokens(theme).line,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .into()
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Plain,
    Digit,
    Edit,
    Op,
    Eq,
}

fn key<'a>(label: &str, k: Key, look: Look) -> Element<'a, Message> {
    let t = text(label.to_string())
        .size(match look {
            Look::Digit | Look::Op => 13.0,
            Look::Eq => 14.0,
            _ => 11.5,
        })
        .font(if look == Look::Op {
            MONO_STRONG
        } else {
            STRONG
        });
    let b = button(container(t).center(Fill))
        .width(Fill)
        .height(28)
        .padding(0)
        .on_press(m(QuickCalcMsg::Key(k)))
        .style(move |theme: &Theme, status| {
            let t = tokens(theme);
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let (fill, border, color) = match look {
                Look::Digit => (
                    if hovered {
                        mix(t.digit, t.text, 0.08)
                    } else {
                        t.digit
                    },
                    t.digit_line,
                    t.text,
                ),
                Look::Eq => (
                    if hovered {
                        mix(t.accent, Color::WHITE, 0.1)
                    } else {
                        t.accent
                    },
                    t.accent,
                    t.on_accent,
                ),
                Look::Edit => (if hovered { t.strong } else { t.weak }, t.warn, t.text),
                Look::Plain | Look::Op => (if hovered { t.strong } else { t.weak }, t.line, t.text),
            };
            button::Style {
                background: Some(Background::Color(fill)),
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: color,
                ..Default::default()
            }
        });
    b.into()
}

fn grid<'a>(keys: Vec<Element<'a, Message>>, columns: usize) -> Element<'a, Message> {
    let mut rows = column![].spacing(3);
    let mut keys = keys.into_iter().peekable();
    while keys.peek().is_some() {
        let cells: Vec<_> = keys.by_ref().take(columns).collect();
        rows = rows.push(row(cells).spacing(3));
    }
    container(rows).padding([6, 2]).into()
}

// ── Sections ────────────────────────────────────────────────────────────

fn section_header<'a>(
    section: Section,
    label: String,
    meta: Option<String>,
    open: bool,
) -> Element<'a, Message> {
    let mut content = row![
        icon(if open { CHEVRON_DOWN } else { CHEVRON_RIGHT }, 10.0, |t| t
            .muted),
        strong(label, 11.0, STRONG).width(Fill),
    ]
    .spacing(6)
    .align_y(iced::Center);
    if let Some(meta) = meta {
        content = content.push(muted(meta, 10.5));
    }
    column![
        container(Space::new().width(Fill).height(1)).style(|theme: &Theme| container::Style {
            background: Some(Background::Color(tokens(theme).line)),
            ..Default::default()
        }),
        button(content)
            .width(Fill)
            .height(24)
            .padding([0, 8])
            .on_press(m(QuickCalcMsg::Toggle(section)))
            .style(|theme: &Theme, status| {
                let t = tokens(theme);
                button::Style {
                    background: Some(Background::Color(
                        if matches!(status, button::Status::Hovered) {
                            t.strong
                        } else {
                            t.weak
                        },
                    )),
                    text_color: t.text,
                    ..Default::default()
                }
            }),
    ]
    .into()
}

fn pad<'a>() -> Element<'a, Message> {
    use Key::*;
    let d = |s: &'static str| key(s, Text(s), Look::Digit);
    let op = |label: &'static str, ins: &'static str| key(label, Text(ins), Look::Op);
    let keys = vec![
        key("MS", MemStore, Look::Plain),
        key("M+", MemAdd, Look::Plain),
        key("MR", MemRecall, Look::Plain),
        key("←", Back, Look::Edit),
        key("C", Clear, Look::Edit),
        key("MC", MemClear, Look::Plain),
        op("(", "("),
        op(")", ")"),
        key("pi", Text("pi"), Look::Plain),
        key("√", Sqrt, Look::Plain),
        key("1/x", Inverse, Look::Plain),
        d("7"),
        d("8"),
        d("9"),
        op("/", " / "),
        key("x²", Square, Look::Plain),
        d("4"),
        d("5"),
        d("6"),
        op("*", " * "),
        key("x³", Cube, Look::Plain),
        d("1"),
        d("2"),
        d("3"),
        op("-", " - "),
        key("xʸ", Text("^"), Look::Plain),
        d("0"),
        d("."),
        key("=", Eval, Look::Eq),
        op("+", " + "),
    ];
    grid(keys, 5)
}

fn scientific<'a>() -> Element<'a, Message> {
    let f = |label: &'static str, ins: &'static str| key(label, Key::Text(ins), Look::Plain);
    grid(
        vec![
            f("sin", "sin("),
            f("cos", "cos("),
            f("tan", "tan("),
            f("√", "sqrt("),
            f("x²", "^2"),
            f("asin", "asin("),
            f("acos", "acos("),
            f("atan", "atan("),
            f("ln", "ln("),
            f("log", "log("),
            f("π", "pi"),
            f("abs", "abs("),
            f("round", "round("),
            f("d2r", "d2r("),
            f("r2d", "r2d("),
        ],
        5,
    )
}

fn point_functions<'a>() -> Element<'a, Message> {
    let punct = |label: &'static str, ins: &'static str| key(label, Key::Text(ins), Look::Op);
    let sig = |name: &'static str, ins: &'static str, signature: &'static str| {
        button(
            column![strong(name, 11.5, STRONG), muted(signature, 10.0)]
                .spacing(2)
                .align_x(iced::Center)
                .width(Fill),
        )
        .width(Fill)
        .height(38)
        .padding([4, 0])
        .on_press(m(QuickCalcMsg::Key(Key::Text(ins))))
        .style(|theme: &Theme, status| {
            let t = tokens(theme);
            button::Style {
                background: Some(Background::Color(
                    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                        t.strong
                    } else {
                        t.weak
                    },
                )),
                border: Border {
                    color: t.line,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: t.text,
                ..Default::default()
            }
        })
        .into()
    };
    grid(
        vec![
            punct("[", "["),
            punct(",", ", "),
            punct("]", "]"),
            sig("dist", "dist(", "(p1, p2)"),
            sig("ang", "ang(", "(p1, p2)"),
            sig("mid", "mid(", "(p1, p2)"),
            sig("xof", "xof(", "(p)"),
            sig("yof", "yof(", "(p)"),
            sig("zof", "zof(", "(p)"),
        ],
        3,
    )
}

fn chip<'a>(label: &'static str, on: bool, msg: QuickCalcMsg) -> Element<'a, Message> {
    button(container(mono(label, 11.0)).center_y(Fill))
        .height(22)
        .padding([0, 8])
        .on_press(m(msg))
        .style(move |theme: &Theme, status| {
            let t = tokens(theme);
            button::Style {
                background: on.then_some(Background::Color(t.accent.scale_alpha(0.16))),
                border: Border {
                    color: if on { t.accent } else { t.line },
                    width: 1.0,
                    radius: 11.0.into(),
                },
                text_color: if on || matches!(status, button::Status::Hovered) {
                    t.text
                } else {
                    t.muted
                },
                ..Default::default()
            }
        })
        .into()
}

fn field_style(theme: &Theme, status: text_input::Status, bad: bool) -> text_input::Style {
    let t = tokens(theme);
    let border = if bad {
        t.danger
    } else if matches!(status, text_input::Status::Focused { .. }) {
        t.accent
    } else {
        t.line
    };
    text_input::Style {
        background: Background::Color(t.field),
        border: Border {
            color: border,
            width: 1.0,
            radius: 3.0.into(),
        },
        icon: t.muted,
        placeholder: t.faint,
        value: t.text,
        selection: t.accent.scale_alpha(0.35),
    }
}

fn units<'a>(p: &'a QuickCalcPalette) -> Element<'a, Message> {
    let cat_label = |c: UnitCat| match c {
        UnitCat::Length => crate::t!("Length"),
        UnitCat::Area => crate::t!("Area"),
        UnitCat::Angle => crate::t!("Angle"),
    };
    let cats = segmented(
        UnitCat::ALL
            .iter()
            .map(|c| {
                segment(
                    cat_label(*c).into_owned(),
                    *c == p.unit_cat,
                    QuickCalcMsg::UnitCat(*c),
                    true,
                )
            })
            .collect(),
    );
    let chips = |selected: &'static str, to: bool| {
        row(p.unit_cat.units().iter().map(|(name, _)| {
            let msg = if to {
                QuickCalcMsg::UnitTo(name)
            } else {
                QuickCalcMsg::UnitFrom(name)
            };
            chip(name, *name == selected, msg)
        }))
        .spacing(4)
    };
    let label = |s: &str| muted(crate::t!(s).into_owned(), 10.5).width(32);
    let result = p
        .converted()
        .map_or_else(|| "—".to_string(), |v| Value::Number(v).display());
    let value_row = row![
        text_input("", &p.unit_value)
            .on_input(|v| m(QuickCalcMsg::UnitValue(v)))
            .font(MONO)
            .size(12)
            .padding([4, 6])
            .width(Length::FillPortion(1))
            .style(|theme, status| field_style(theme, status, false)),
        muted(p.unit_from, 11.0).font(MONO).width(32),
        tip(
            button(icon(SWAP, 14.0, |t| t.muted))
                .padding(4)
                .on_press(m(QuickCalcMsg::UnitSwap))
                .style(|theme: &Theme, status| {
                    let mut s = plain_button_style(theme, status);
                    s.border.width = 0.0;
                    s
                }),
            crate::t!("Swap units").into_owned(),
        ),
        container(strong(result, 13.0, MONO_STRONG)).align_right(Length::FillPortion(1)),
        muted(p.unit_to, 11.0).font(MONO).width(32),
    ]
    .spacing(6)
    .align_y(iced::Center);
    let insert: Element<'a, Message> = match p.converted_literal() {
        Some(lit) => text_button(
            row![text(crate::t!("Insert")).size(11), mono(lit, 11.0)].spacing(4),
            QuickCalcMsg::UnitInsert,
        ),
        None => Space::new().into(),
    };
    column![
        cats,
        row![label("From"), chips(p.unit_from, false)]
            .spacing(6)
            .align_y(iced::Center),
        row![label("To"), chips(p.unit_to, true)]
            .spacing(6)
            .align_y(iced::Center),
        value_row,
        container(insert).align_right(Fill),
    ]
    .spacing(8)
    .padding([8, 2])
    .into()
}

fn var_row<'a>(name: &str, value: &Value, scope: VarScope) -> Element<'a, Message> {
    let name = name.to_string();
    let insert = button(
        row![
            text(name.clone())
                .size(12)
                .font(MONO)
                .style(|theme: &Theme| text::Style {
                    color: Some(tokens(theme).accent),
                }),
            Space::new().width(Fill),
            mono(value.display(), 12.0),
        ]
        .align_y(iced::Center),
    )
    .width(Fill)
    .height(24)
    .padding([0, 6])
    .on_press(m(QuickCalcMsg::VarInsert(name.clone())))
    .style(|theme: &Theme, status| {
        let t = tokens(theme);
        button::Style {
            background: matches!(status, button::Status::Hovered)
                .then_some(Background::Color(t.weak)),
            text_color: t.text,
            ..Default::default()
        }
    });
    let delete = button(icon(DELETE, 11.0, |t| t.faint))
        .padding(4)
        .on_press(m(QuickCalcMsg::VarDelete(scope, name.clone())))
        .style(|theme: &Theme, status| {
            let mut s = plain_button_style(theme, status);
            s.border.width = 0.0;
            s
        });
    row![
        tip(insert, crate::t!("Insert into the expression").into_owned()),
        tip(delete, crate::tf!("Delete {}", name).into_owned()),
    ]
    .spacing(2)
    .align_y(iced::Center)
    .into()
}

fn variables<'a>(p: &'a QuickCalcPalette, drawing: &[(String, f64)]) -> Element<'a, Message> {
    let heading = |title: &str, note: &str| {
        row![
            text(crate::t!(title).into_owned()).size(10.5).width(Fill),
            muted(crate::t!(note).into_owned(), 10.5),
        ]
        .padding([4, 6])
    };
    let mut list = column![heading("This drawing", "saved with the drawing")];
    for (name, value) in drawing {
        if !p.session.contains_key(name) {
            list = list.push(var_row(name, &Value::Number(*value), VarScope::Drawing));
        }
    }
    let mut session: Vec<_> = p.session.iter().collect();
    session.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in session {
        list = list.push(var_row(name, value, VarScope::Drawing));
    }
    list = list.push(heading("My constants", "every drawing"));
    for (name, value) in &p.constants {
        list = list.push(var_row(name, &Value::Number(*value), VarScope::Constant));
    }
    list.push(
        container(text_button(
            row![
                icon(PLUS, 11.0, |t| t.text),
                text(crate::t!("New variable")).size(11)
            ]
            .spacing(5)
            .align_y(iced::Center),
            QuickCalcMsg::NewVar,
        ))
        .padding([6, 6]),
    )
    .padding([4, 0])
    .into()
}

// ── The pallet ──────────────────────────────────────────────────────────

/// Everything the view needs from the app besides the pallet itself.
pub struct Inputs {
    /// The drawing's variables with their values.
    pub drawing: Vec<(String, f64)>,
    /// Millimetres per drawing unit.
    pub base_unit: f64,
    /// The drawing unit's short name (`mm`, `m`, `in`, …).
    pub unit_label: &'static str,
    /// The step of the running QuickCalc pick, if one is running.
    pub pick_step: Option<u8>,
    /// The app's theme, for the colours inside the error echo.
    pub theme: Theme,
}

/// History rows the layout always keeps room for; the sections scroll
/// instead of squeezing the history below this.
const MIN_HISTORY_ROWS: f32 = 4.0;
const HISTORY_ROW_H: f32 = 22.0;
/// Space between an embedded scrollbar and the content it scrolls.
const SCROLLBAR_GAP: f32 = 4.0;

pub fn view<'a>(
    p: &'a QuickCalcPalette,
    inputs: Inputs,
    width: f32,
    chrome: Chrome,
) -> Element<'a, Message> {
    let title_bar = crate::ui::dock::title_bar(
        PanelId::QuickCalc,
        crate::t!("QuickCalc").into_owned(),
        chrome,
    );
    let body = iced::widget::responsive(move |size| body(p, &inputs, size.height));
    crate::ui::dock::frame(title_bar, body, width)
}

fn body<'a>(p: &'a QuickCalcPalette, inputs: &Inputs, height: f32) -> Element<'a, Message> {
    let ctx = p.context(&inputs.drawing, inputs.base_unit);
    let preview = p.preview(&ctx);
    let picking = inputs.pick_step.zip(p.pick);
    let active = |kind: PickKind| picking.is_some_and(|(_, k)| k == kind);

    // Toolbar.
    let mode_toggle = segmented(vec![
        segment(
            "DEG".into(),
            p.mode == AngleMode::Degrees,
            QuickCalcMsg::SetMode(AngleMode::Degrees),
            false,
        ),
        segment(
            "RAD".into(),
            p.mode == AngleMode::Radians,
            QuickCalcMsg::SetMode(AngleMode::Radians),
            false,
        ),
    ]);
    let divider =
        container(Space::new().width(1).height(16)).style(|theme: &Theme| container::Style {
            background: Some(Background::Color(tokens(theme).line)),
            ..Default::default()
        });
    let toolbar = row![
        tool(
            POINT,
            crate::t!("Get point").into_owned(),
            QuickCalcMsg::Pick(PickKind::Point),
            active(PickKind::Point)
        ),
        tool(
            DISTANCE,
            crate::t!("Distance between two points").into_owned(),
            QuickCalcMsg::Pick(PickKind::Distance),
            active(PickKind::Distance)
        ),
        tool(
            ANGLE,
            crate::t!("Angle of a line").into_owned(),
            QuickCalcMsg::Pick(PickKind::Angle),
            active(PickKind::Angle)
        ),
        container(divider).padding([0, 4]),
        tool(
            crate::ui::icons::TRASH,
            crate::t!("Clear history").into_owned(),
            QuickCalcMsg::ClearHistory,
            false
        ),
        tool(
            COPY,
            crate::t!("Copy result").into_owned(),
            QuickCalcMsg::Copy,
            false
        ),
        tool(
            SEND,
            crate::t!("Paste to command line").into_owned(),
            QuickCalcMsg::PasteToCommandLine,
            false
        ),
        Space::new().width(Fill),
        mode_toggle,
    ]
    .spacing(2)
    .align_y(iced::Center);

    // History, newest at the bottom next to the input.
    // The empty state, or the rows; the list around the rows is built once
    // the layout knows whether they overflow (see below).
    let (empty_state, history_rows): (Option<Element<'a, Message>>, Vec<Element<'a, Message>>) =
        if p.history.is_empty() {
            (Some(container(
            column![
                icon(ICON, 28.0, |t| t.faint),
                text(crate::t!("Results collect here")).size(11.5),
                muted(
                    crate::t!(
                        "Type an expression below, or pick a point, distance or angle from the drawing."
                    )
                    .into_owned(),
                    11.5
                )
                .center(),
            ]
            .spacing(8)
            .align_x(iced::Center),
        )
        .padding([24, 20])
        .center(Fill)
        .into()), Vec::new())
        } else {
            let rows = p.history.iter().enumerate().map(|(i, e)| {
                let expr = button(
                    muted(e.expr.clone(), 11.5)
                        .font(MONO)
                        .wrapping(Wrapping::None)
                        .ellipsis(Ellipsis::End),
                )
                .width(Fill)
                .padding(0)
                .on_press(m(QuickCalcMsg::HistoryLoad(i)))
                .style(button::text);
                let result = button(text(e.value.display()).size(12.5).font(MONO_STRONG))
                    .padding(0)
                    .on_press(m(QuickCalcMsg::HistoryCopy(i)))
                    .style(|theme: &Theme, status| {
                        let t = tokens(theme);
                        button::Style {
                            text_color: if matches!(status, button::Status::Hovered) {
                                t.accent
                            } else {
                                t.text
                            },
                            ..Default::default()
                        }
                    });
                container(
                    row![
                        tip(expr, crate::t!("Load into the input").into_owned()),
                        tip(result, crate::t!("Copy result").into_owned()),
                    ]
                    .spacing(10)
                    .align_y(iced::Center),
                )
                .padding([3, 4])
                .into()
            });
            (None, rows.collect())
        };

    // Input, with the pick banner above it and the preview or error below.
    let mut input_block = column![].spacing(6);
    if let Some((step, kind)) = picking {
        let (title, prompt) = match (kind, step) {
            (PickKind::Point, _) => (
                crate::t!("Get point"),
                crate::t!("Specify a point in the drawing"),
            ),
            (PickKind::Distance, 1) => (
                crate::t!("Distance"),
                crate::t!("Specify first point in the drawing"),
            ),
            (PickKind::Distance, _) => (
                crate::t!("Distance"),
                crate::t!("Specify second point in the drawing"),
            ),
            (PickKind::Angle, 1) => (
                crate::t!("Angle"),
                crate::t!("Specify first point in the drawing"),
            ),
            (PickKind::Angle, _) => (
                crate::t!("Angle"),
                crate::t!("Specify second point in the drawing"),
            ),
        };
        let mut heading = row![text(title.into_owned()).size(11.5)].spacing(6);
        if kind.steps() > 1 {
            heading = heading.push(muted(
                crate::tf!("step {} of {}", step, kind.steps()).into_owned(),
                11.5,
            ));
        }
        input_block = input_block.push(
            container(
                row![
                    icon(POINT, 16.0, |t| t.accent),
                    column![heading, text(prompt.into_owned()).size(11.5)]
                        .spacing(1)
                        .width(Fill),
                    text_button(text(crate::t!("Cancel")).size(11), QuickCalcMsg::CancelPick),
                ]
                .spacing(8)
                .align_y(iced::Center),
            )
            .padding([7, 8])
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(tokens(theme).accent.scale_alpha(0.16))),
                border: Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
    }
    let show_error = matches!(preview, Preview::Err(_)) && picking.is_none() && p.error_shown();
    let bad = show_error;
    let placeholder = if p.new_var_hint {
        crate::t!("Type name = value, then Enter")
    } else if p.history.is_empty() {
        crate::t!("Expression, e.g. 2*300 + 90")
    } else {
        crate::t!("Expression, or start with * to reuse ans")
    };
    input_block = input_block.push(
        text_input(&placeholder, &p.input)
            .id(iced::widget::Id::new(INPUT_ID))
            .on_input(|v| m(QuickCalcMsg::Input(v)))
            .on_submit(m(QuickCalcMsg::Submit))
            .font(MONO)
            .size(13)
            .padding([6, 8])
            .style(move |theme, status| field_style(theme, status, bad)),
    );
    let trimmed = p.input.trim();
    match &preview {
        Preview::Err(e) if show_error => {
            let start = e.span.start.min(p.input.len());
            let end = e.span.end.clamp(start, p.input.len());
            let bad_text = match &p.input[start..end] {
                "" => " ".to_string(),
                s => s.to_string(),
            };
            let t = tokens(&inputs.theme);
            let echo = rich_text::<(), _, _, _>(vec![
                Span::new(p.input[..start].to_string()).color(t.muted),
                Span::new(bad_text)
                    .color(t.danger_text)
                    .background(t.danger.scale_alpha(0.18))
                    .underline(true),
                Span::new(p.input[end..].to_string()).color(t.muted),
            ])
            .size(12)
            .font(MONO)
            .wrapping(Wrapping::None);
            input_block = input_block.push(
                column![
                    echo,
                    row![
                        icon(ERROR, 13.0, |t| t.danger_text),
                        text(e.message.clone())
                            .size(11.5)
                            .style(|theme: &Theme| text::Style {
                                color: Some(tokens(theme).danger_text),
                            }),
                    ]
                    .spacing(6)
                    .align_y(iced::Center),
                ]
                .spacing(3),
            );
        }
        _ if picking.is_some() => {}
        Preview::Ok(value) => {
            let lead = if let Some(name) = assignment_target(trimmed) {
                crate::tf!("sets {}", name).into_owned()
            } else if continues_ans(trimmed) {
                format!("ans {trimmed}")
            } else if has_length_suffix(trimmed) {
                crate::tf!("in {}", inputs.unit_label).into_owned()
            } else {
                String::new()
            };
            input_block = input_block.push(preview_line(lead, format!("= {}", value.display())));
        }
        _ => {
            let lead = p
                .ans
                .map(|a| format!("ans = {}", a.display()))
                .unwrap_or_default();
            input_block = input_block.push(preview_line(lead, String::new()));
        }
    }

    // Sections.
    let mode_short = match p.mode {
        AngleMode::Degrees => "DEG",
        AngleMode::Radians => "RAD",
    };
    let mut sections = column![];
    for section in Section::ALL {
        let open = p.is_open(section);
        let (label, meta) = match section {
            Section::Pad => (crate::t!("Number pad"), None),
            Section::Scientific => (crate::t!("Scientific"), Some(mode_short.to_string())),
            Section::Points => (crate::t!("Point functions"), None),
            Section::Units => (
                crate::t!("Units"),
                Some(format!("{} → {}", p.unit_from, p.unit_to)),
            ),
            Section::Variables => (
                crate::t!("Variables"),
                Some((inputs.drawing.len() + p.session.len() + p.constants.len()).to_string()),
            ),
        };
        sections = sections.push(section_header(section, label.into_owned(), meta, open));
        if open {
            sections = sections.push(match section {
                Section::Pad => pad(),
                Section::Scientific => scientific(),
                Section::Points => point_functions(),
                Section::Units => units(p),
                Section::Variables => variables(p, &inputs.drawing),
            });
        }
    }

    // Footer.
    let mode_word = match p.mode {
        AngleMode::Degrees => crate::t!("Degrees"),
        AngleMode::Radians => crate::t!("Radians"),
    };
    let mut left = row![
        muted(
            crate::tf!("Units: {}", inputs.unit_label).into_owned(),
            10.5
        ),
        muted("·", 10.5),
        muted(mode_word.into_owned(), 10.5),
    ]
    .spacing(6)
    .align_y(iced::Center);
    if let Some(mem) = p.memory {
        left = left.push(tip(
            container(mono(format!("M {}", Value::Number(mem).display()), 10.5))
                .padding([0, 5])
                .style(|theme: &Theme| container::Style {
                    border: Border {
                        color: tokens(theme).line,
                        width: 1.0,
                        radius: 3.0.into(),
                    },
                    ..Default::default()
                }),
            crate::t!("Value in memory").into_owned(),
        ));
    }
    let (status, flashing) = match &p.flash {
        Some((msg, _)) => (msg.clone(), true),
        None => (
            crate::t!("Enter evaluates · ↑↓ history").into_owned(),
            false,
        ),
    };
    let footer = container(
        row![
            left,
            Space::new().width(Fill),
            text(status)
                .size(10.5)
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .style(move |theme: &Theme| {
                    let t = tokens(theme);
                    text::Style {
                        color: Some(if flashing { t.accent } else { t.muted }),
                    }
                }),
        ]
        .spacing(6)
        .align_y(iced::Center),
    )
    .height(22)
    .padding([0, 6])
    .center_y(22)
    .style(|theme: &Theme| {
        let t = tokens(theme);
        container::Style {
            background: Some(Background::Color(t.weak)),
            border: Border {
                color: t.line,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        }
    });

    // Room for the sections: what is left after the toolbar, the input, the
    // footer, the gaps and the history's minimum. Past that they scroll.
    let input_h = 30.0
        + 6.0
        + if show_error { 38.0 } else { 18.0 }
        + if picking.is_some() { 52.0 } else { 0.0 };
    let history_min = MIN_HISTORY_ROWS * HISTORY_ROW_H;
    let fixed = 28.0 + input_h + 22.0 + 4.0 * 6.0;
    let sections_cap = (height - fixed - history_min).max(0.0);
    let natural = sections_height(p, inputs.drawing.len());
    // A scrollbar that shows takes its own room beside the content
    // (`spacing` embeds it) instead of covering the keys or the results.
    let sections: Element<'a, Message> = if natural <= sections_cap {
        sections.into()
    } else {
        scrollable(sections)
            .spacing(SCROLLBAR_GAP)
            .height(sections_cap)
            .into()
    };
    let history: Element<'a, Message> = match empty_state {
        Some(empty) => empty,
        None => {
            // Newest at the bottom, right above the input: a short history
            // sits on the bottom edge, a long one scrolls up from it.
            let room = height - fixed - natural.min(sections_cap);
            let overflows = history_rows.len() as f32 * HISTORY_ROW_H > room;
            let mut list = scrollable(column(history_rows))
                .anchor_bottom()
                .height(Length::Shrink);
            if overflows {
                list = list.spacing(SCROLLBAR_GAP);
            }
            container(list).align_bottom(Fill).width(Fill).into()
        }
    };

    column![
        toolbar,
        container(history).height(Fill),
        input_block,
        sections,
        footer,
    ]
    .spacing(6)
    .height(Fill)
    .into()
}

/// The sections' height when laid out in full: every header plus the open
/// bodies, from the fixed key and row sizes they are built with.
fn sections_height(p: &QuickCalcPalette, drawing_vars: usize) -> f32 {
    const HEADER: f32 = 25.0;
    let grid = |rows: f32, row_h: f32| rows * row_h + (rows - 1.0) * 3.0 + 12.0;
    Section::ALL
        .iter()
        .map(|section| {
            HEADER
                + if p.is_open(*section) {
                    match section {
                        Section::Pad => grid(6.0, 28.0),
                        Section::Scientific => grid(3.0, 28.0),
                        Section::Points => 28.0 + 2.0 * 38.0 + 2.0 * 3.0 + 12.0,
                        Section::Units => 24.0 + 2.0 * 22.0 + 28.0 + 26.0 + 4.0 * 8.0 + 16.0,
                        Section::Variables => {
                            let rows = drawing_vars + p.session.len() + p.constants.len();
                            2.0 * 22.0 + rows as f32 * 24.0 + 38.0 + 8.0
                        }
                    }
                } else {
                    0.0
                }
        })
        .sum()
}

fn preview_line<'a>(lead: String, value: String) -> Element<'a, Message> {
    row![
        muted(lead, 11.0)
            .font(MONO)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .width(Fill),
        strong(value, 14.0, MONO_STRONG),
    ]
    .spacing(8)
    .height(Length::Fixed(18.0))
    .align_y(iced::Center)
    .into()
}
