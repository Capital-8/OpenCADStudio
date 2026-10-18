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
const CLEAR: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/clear.svg");
const COPY: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/copy.svg");
const SEND: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/send.svg");
const CHEVRON_DOWN: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/chevron_down.svg");
const CHEVRON_RIGHT: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/chevron_right.svg");
const SWAP: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/swap.svg");
const DELETE: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/delete.svg");
const PLUS: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/plus.svg");
const ERROR: &[u8] = include_bytes!("../../../../assets/icons/quickcalc/error.svg");

const MONO: Font = Font::MONOSPACE;

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
        on_accent: p.primary.base.text,
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
            mix(bg, Color::WHITE, 0.15)
        } else {
            Color::WHITE
        },
        digit_line: if dark {
            mix(bg, Color::WHITE, 0.22)
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

fn tip<'a>(content: impl Into<Element<'a, Message>>, label: String) -> Element<'a, Message> {
    tooltip(content, text(label).size(10), tooltip::Position::Bottom)
        .gap(4)
        .into()
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
    let b = button(container(text(label).size(10)).center_x(Fill))
        .padding([3, 7])
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

fn key<'a>(label: &str, k: Key, look: Look, hint: Option<String>) -> Element<'a, Message> {
    let mut t = text(label.to_string()).size(match look {
        Look::Digit | Look::Op => 13.0,
        Look::Eq => 14.0,
        _ => 11.5,
    });
    if look == Look::Op {
        t = t.font(MONO);
    }
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
                Look::Op => (if hovered { t.strong } else { t.weak }, t.line, t.accent),
                Look::Plain => (if hovered { t.strong } else { t.weak }, t.line, t.text),
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
    match hint {
        Some(h) => tip(b, h),
        None => b.into(),
    }
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
        text(label).size(11).width(Fill),
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
    let t = |s: &str| crate::t!(s).into_owned();
    let d = |s: &'static str| key(s, Text(s), Look::Digit, None);
    let op = |label: &'static str, ins: &'static str| key(label, Text(ins), Look::Op, None);
    let keys = vec![
        key("MS", MemStore, Look::Plain, Some(t("Memory store"))),
        key("M+", MemAdd, Look::Plain, Some(t("Add to memory"))),
        key("MR", MemRecall, Look::Plain, Some(t("Memory recall"))),
        key("←", Back, Look::Edit, Some(t("Backspace"))),
        key("C", Clear, Look::Edit, Some(t("Clear input"))),
        key("MC", MemClear, Look::Plain, Some(t("Memory clear"))),
        op("(", "("),
        op(")", ")"),
        key("pi", Text("pi"), Look::Plain, Some(t("Pi"))),
        key("√", Sqrt, Look::Plain, Some(t("Square root of the input"))),
        key(
            "1/x",
            Inverse,
            Look::Plain,
            Some(t("Reciprocal of the input")),
        ),
        d("7"),
        d("8"),
        d("9"),
        op("/", " / "),
        key("x²", Square, Look::Plain, Some(t("Square the input"))),
        d("4"),
        d("5"),
        d("6"),
        op("*", " * "),
        key("x³", Cube, Look::Plain, Some(t("Cube the input"))),
        d("1"),
        d("2"),
        d("3"),
        op("-", " - "),
        key("xʸ", Text("^"), Look::Plain, Some(t("Raise to a power"))),
        d("0"),
        d("."),
        key("=", Eval, Look::Eq, Some(t("Evaluate the input"))),
        op("+", " + "),
    ];
    grid(keys, 5)
}

fn scientific<'a>() -> Element<'a, Message> {
    let f = |label: &'static str, ins: &'static str, hint: &str| {
        key(
            label,
            Key::Text(ins),
            Look::Plain,
            Some(crate::t!(hint).into_owned()),
        )
    };
    grid(
        vec![
            f("sin", "sin(", "Sine"),
            f("cos", "cos(", "Cosine"),
            f("tan", "tan(", "Tangent of an angle"),
            f("√", "sqrt(", "Square root"),
            f("x²", "^2", "Squared"),
            f("asin", "asin(", "Arcsine"),
            f("acos", "acos(", "Arccosine"),
            f("atan", "atan(", "Arctangent"),
            f("ln", "ln(", "Natural logarithm"),
            f("log", "log(", "Base-10 logarithm"),
            f("π", "pi", "Pi"),
            f("abs", "abs(", "Absolute value"),
            f("round", "round(", "Round"),
            f("d2r", "d2r(", "Degrees to radians"),
            f("r2d", "r2d(", "Radians to degrees"),
        ],
        5,
    )
}

fn point_functions<'a>() -> Element<'a, Message> {
    let punct = |label: &'static str, ins: &'static str, hint: &str| {
        key(
            label,
            Key::Text(ins),
            Look::Op,
            Some(crate::t!(hint).into_owned()),
        )
    };
    let sig = |name: &'static str, ins: &'static str, signature: &'static str, hint: &str| {
        let b = button(
            column![text(name).size(11.5), muted(signature, 10.0)]
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
        });
        tip(b, crate::t!(hint).into_owned())
    };
    grid(
        vec![
            punct("[", "[", "Start a point"),
            punct(",", ", ", "Separate coordinates"),
            punct("]", "]", "End a point"),
            sig("dist", "dist(", "(p1, p2)", "Distance between two points"),
            sig("ang", "ang(", "(p1, p2)", "Angle of the line from p1 to p2"),
            sig("mid", "mid(", "(p1, p2)", "Midpoint of two points"),
            sig("xof", "xof(", "(p)", "X coordinate of a point"),
            sig("yof", "yof(", "(p)", "Y coordinate of a point"),
            sig("zof", "zof(", "(p)", "Z coordinate of a point"),
        ],
        3,
    )
}

fn chip<'a>(label: &'static str, on: bool, msg: QuickCalcMsg) -> Element<'a, Message> {
    button(mono(label, 11.0))
        .padding([2, 8])
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
    let label = |s: &str| muted(crate::t!(s).into_owned(), 10.5).width(30);
    let result = p
        .converted()
        .map_or_else(|| "—".to_string(), |v| Value::Number(v).display());
    let value_row = row![
        text_input("", &p.unit_value)
            .on_input(|v| m(QuickCalcMsg::UnitValue(v)))
            .font(MONO)
            .size(12)
            .padding([4, 6])
            .width(Fill)
            .style(|theme, status| field_style(theme, status, false)),
        muted(p.unit_from, 11.0).font(MONO).width(30),
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
        container(mono(result, 13.0)).align_right(Fill),
        muted(p.unit_to, 11.0).font(MONO).width(30),
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
pub struct Inputs<'a> {
    /// The drawing's variables with their values.
    pub drawing: &'a [(String, f64)],
    /// Millimetres per drawing unit.
    pub base_unit: f64,
    /// The drawing unit's short name (`mm`, `m`, `in`, …).
    pub unit_label: &'a str,
    /// The step of the running QuickCalc pick, if one is running.
    pub pick_step: Option<u8>,
    /// The app's theme, for the colours inside the error echo.
    pub theme: &'a Theme,
}

pub fn view<'a>(
    p: &'a QuickCalcPalette,
    inputs: Inputs<'_>,
    width: f32,
    chrome: Chrome,
) -> Element<'a, Message> {
    let title_bar = crate::ui::dock::title_bar(
        PanelId::QuickCalc,
        crate::t!("QuickCalc").into_owned(),
        chrome,
    );
    let ctx = p.context(inputs.drawing, inputs.base_unit);
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
            CLEAR,
            crate::t!("Clear input").into_owned(),
            QuickCalcMsg::Clear,
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
    let mut header = row![muted(crate::t!("History").into_owned(), 10.5).width(Fill)];
    if !p.history.is_empty() {
        header = header.push(
            button(muted(crate::t!("Clear history").into_owned(), 10.5))
                .padding(0)
                .on_press(m(QuickCalcMsg::ClearHistory))
                .style(button::text),
        );
    }
    let history: Element<'a, Message> = if p.history.is_empty() {
        container(
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
        .into()
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
            let result = button(mono(e.value.display(), 12.5))
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
        scrollable(column(rows)).anchor_bottom().height(Fill).into()
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
    let bad = matches!(preview, Preview::Err(_)) && picking.is_none();
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
        Preview::Err(e) if picking.is_none() => {
            let start = e.span.start.min(p.input.len());
            let end = e.span.end.clamp(start, p.input.len());
            let bad_text = match &p.input[start..end] {
                "" => " ".to_string(),
                s => s.to_string(),
            };
            let t = tokens(inputs.theme);
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
                Section::Variables => variables(p, inputs.drawing),
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

    let body = column![
        toolbar,
        column![header, history].spacing(2).height(Fill),
        input_block,
        sections,
        footer,
    ]
    .spacing(6)
    .height(Fill);
    crate::ui::dock::frame(title_bar, body, width)
}

fn preview_line<'a>(lead: String, value: String) -> Element<'a, Message> {
    row![
        muted(lead, 11.0)
            .font(MONO)
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::End)
            .width(Fill),
        mono(value, 14.0),
    ]
    .spacing(8)
    .height(Length::Fixed(18.0))
    .align_y(iced::Center)
    .into()
}
