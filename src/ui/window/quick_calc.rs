//! The QuickCalc pallet (QUICKCALC / QC): an expression field on the shared
//! calculator engine (`app::calc`) with its history, number pad, scientific
//! and point functions, a unit converter, variables, and picks that take a
//! point, distance or angle from the drawing.
//!
//! The pallet's state and everything it does without the drawing live here;
//! the app side (`app::quick_calc`) adds the drawing variables, the
//! clipboard, the command line and the picks.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::app::calc::{self, AngleMode, CalcContext, CalcError, Value};
use crate::modules::draw::inquiry::calc_pick::PickKind;

/// The expression field, so the app can focus it.
pub const INPUT_ID: &str = "quick-calc-input";

/// How long a footer status ("Copied 870") stays.
const FLASH_SECS: f32 = 2.2;

/// The collapsible sections, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Pad,
    Scientific,
    Points,
    Units,
    Variables,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Pad,
        Section::Scientific,
        Section::Points,
        Section::Units,
        Section::Variables,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

/// What the unit converter converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitCat {
    Length,
    Area,
    Angle,
}

impl UnitCat {
    pub const ALL: [UnitCat; 3] = [UnitCat::Length, UnitCat::Area, UnitCat::Angle];

    /// The units, each with its size in the category's base unit (mm, m², °).
    pub fn units(self) -> &'static [(&'static str, f64)] {
        match self {
            UnitCat::Length => &[
                ("mm", 1.0),
                ("cm", 10.0),
                ("m", 1000.0),
                ("km", 1_000_000.0),
                ("in", 25.4),
                ("ft", 304.8),
            ],
            UnitCat::Area => &[
                ("mm²", 1e-6),
                ("cm²", 1e-4),
                ("m²", 1.0),
                ("ha", 10_000.0),
                ("ft²", 0.092_903_04),
            ],
            UnitCat::Angle => &[
                ("deg", 1.0),
                ("rad", 180.0 / std::f64::consts::PI),
                ("grad", 0.9),
            ],
        }
    }

    /// The pair the converter starts on.
    fn defaults(self) -> (&'static str, &'static str) {
        match self {
            UnitCat::Length => ("mm", "m"),
            UnitCat::Area => ("mm²", "m²"),
            UnitCat::Angle => ("deg", "rad"),
        }
    }

    fn size(self, unit: &str) -> Option<f64> {
        self.units()
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, size)| *size)
    }
}

/// A number pad, scientific or point key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Appends its text to the input.
    Text(&'static str),
    Back,
    Clear,
    Eval,
    /// √, 1/x, x², x³ wrap the whole input (or `ans` when it is empty).
    Sqrt,
    Inverse,
    Square,
    Cube,
    MemStore,
    MemAdd,
    MemRecall,
    MemClear,
}

/// Where a variable is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarScope {
    /// In the drawing's parameter table (saved with the drawing); point
    /// variables, which a parameter cannot hold, last for the session.
    Drawing,
    /// "My constants", in the user settings: every drawing sees them.
    Constant,
}

#[derive(Debug, Clone)]
pub enum QuickCalcMsg {
    Input(String),
    /// Enter: evaluate, add to the history, set `ans`.
    Submit,
    /// Ctrl+Enter, or the toolbar button: the current value to the command line.
    PasteToCommandLine,
    Copy,
    Clear,
    Pick(PickKind),
    CancelPick,
    /// A pick finished (from `CmdResult::QuickCalcPick`).
    Picked(Value),
    SetMode(AngleMode),
    ClearHistory,
    /// A history row's expression: load it into the input.
    HistoryLoad(usize),
    /// A history row's result: copy it.
    HistoryCopy(usize),
    /// ↑ (`true`) / ↓ through the history while the input has focus.
    HistoryStep(bool),
    Key(Key),
    Toggle(Section),
    UnitCat(UnitCat),
    UnitFrom(&'static str),
    UnitTo(&'static str),
    UnitValue(String),
    UnitSwap,
    UnitInsert,
    VarInsert(String),
    VarDelete(VarScope, String),
    NewVar,
    /// The footer status timer ran out.
    FlashExpired,
}

/// The settings kept with the user's profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuickCalcSettings {
    /// "My constants", in the order they were made.
    pub constants: Vec<(String, f64)>,
    /// Which sections are open, in [`Section::ALL`] order.
    pub open: [bool; 5],
    pub radians: bool,
}

impl Default for QuickCalcSettings {
    fn default() -> Self {
        Self {
            constants: Vec::new(),
            open: [true, false, false, false, false],
            radians: false,
        }
    }
}

/// One evaluated line. The value keeps full precision; only the view rounds.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub expr: String,
    pub value: Value,
}

/// What the input currently evaluates to.
#[derive(Debug, Clone, PartialEq)]
pub enum Preview {
    Empty,
    Ok(Value),
    Err(CalcError),
}

/// A finished evaluation the app still has to act on.
#[derive(Debug, Clone, PartialEq)]
pub struct Submitted {
    pub value: Value,
    /// `name = …`: the variable it sets.
    pub assign: Option<String>,
}

#[derive(Debug)]
pub struct QuickCalcPalette {
    pub show: bool,
    pub input: String,
    pub history: Vec<Entry>,
    /// The history row ↑/↓ is on.
    pub hist_idx: Option<usize>,
    pub ans: Option<Value>,
    pub mode: AngleMode,
    /// Point variables, which a drawing parameter cannot hold.
    pub session: HashMap<String, Value>,
    pub constants: Vec<(String, f64)>,
    pub memory: Option<f64>,
    pub open: [bool; 5],
    /// The pick the pallet started (its command may since have ended).
    pub pick: Option<PickKind>,
    pub flash: Option<(String, iced::time::Instant)>,
    /// The placeholder "New variable" asks for, until the next evaluation.
    pub new_var_hint: bool,
    pub unit_cat: UnitCat,
    pub unit_from: &'static str,
    pub unit_to: &'static str,
    pub unit_value: String,
    /// The user is typing in the input (set by input, cleared by any press
    /// elsewhere). Escape clears the input only then, since a focused
    /// field drops its focus before the Escape shortcut arrives.
    pub typing: bool,
    /// The input as it was when Enter failed on it. Its error shows only
    /// while the input is unchanged, so a half-typed `1.2m / ` is not
    /// flagged before the user asks for the result.
    pub failed_input: Option<String>,
}

impl Default for QuickCalcPalette {
    fn default() -> Self {
        let (unit_from, unit_to) = UnitCat::Length.defaults();
        let settings = QuickCalcSettings::default();
        Self {
            show: false,
            input: String::new(),
            history: Vec::new(),
            hist_idx: None,
            ans: None,
            mode: AngleMode::Degrees,
            session: HashMap::new(),
            constants: settings.constants,
            memory: None,
            open: settings.open,
            pick: None,
            flash: None,
            new_var_hint: false,
            unit_cat: UnitCat::Length,
            unit_from,
            unit_to,
            unit_value: "1".into(),
            typing: false,
            failed_input: None,
        }
    }
}

/// Whether `input` starts with `* / ^`, so it continues from `ans`.
pub fn continues_ans(input: &str) -> bool {
    input.trim_start().starts_with(['*', '/', '^'])
}

/// `name` when `input` is `name = …`.
pub fn assignment_target(input: &str) -> Option<&str> {
    let (lhs, rhs) = input.split_once('=')?;
    let name = lhs.trim();
    let starts = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_');
    (starts && name.chars().all(|c| c.is_alphanumeric() || c == '_') && !rhs.starts_with('='))
        .then_some(name)
}

/// Whether `input` has a length suffix, so its value was converted.
pub fn has_length_suffix(input: &str) -> bool {
    let bytes = input.as_bytes();
    if input.contains(['\'', '"']) {
        return true;
    }
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() || bytes[i] == b'.' {
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let suffix = input[start..i].to_ascii_lowercase();
            if matches!(suffix.as_str(), "mm" | "cm" | "m" | "km" | "in" | "ft") {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// A value as the clipboard and the command line get it: full precision,
/// and a point as the `x,y,z` a coordinate prompt reads.
pub fn full_text(value: &Value) -> String {
    match value {
        Value::Number(n) => n.to_string(),
        Value::Point(p) if p.z == 0.0 => format!("{},{}", p.x, p.y),
        Value::Point(p) => format!("{},{},{}", p.x, p.y, p.z),
    }
}

impl QuickCalcPalette {
    pub fn settings(&self) -> QuickCalcSettings {
        QuickCalcSettings {
            constants: self.constants.clone(),
            open: self.open,
            radians: self.mode == AngleMode::Radians,
        }
    }

    pub fn apply_settings(&mut self, s: &QuickCalcSettings) {
        self.constants = s
            .constants
            .iter()
            .filter(|(name, value)| !calc::is_reserved_name(name) && value.is_finite())
            .cloned()
            .collect();
        self.open = s.open;
        self.mode = if s.radians {
            AngleMode::Radians
        } else {
            AngleMode::Degrees
        };
    }

    pub fn is_open(&self, section: Section) -> bool {
        self.open[section.index()]
    }

    /// The evaluation context: constants, then the drawing's variables, then
    /// this session's point variables — a later one wins on a shared name.
    pub fn context(&self, drawing: &[(String, f64)], base_unit: f64) -> CalcContext {
        let mut ctx = CalcContext::new(self.mode, base_unit);
        for (name, value) in self.constants.iter().chain(drawing) {
            ctx.vars.insert(name.clone(), Value::Number(*value));
        }
        for (name, value) in &self.session {
            ctx.vars.insert(name.clone(), *value);
        }
        ctx.ans = self.ans;
        ctx
    }

    /// Whether the error of the current input should show: only after Enter
    /// failed on exactly this text.
    pub fn error_shown(&self) -> bool {
        self.failed_input.as_deref() == Some(self.input.as_str())
    }

    pub fn preview(&self, ctx: &CalcContext) -> Preview {
        if self.input.trim().is_empty() {
            return Preview::Empty;
        }
        match calc::evaluate(&self.input, &mut ctx.clone()) {
            Ok(value) => Preview::Ok(value),
            Err(error) => Preview::Err(error),
        }
    }

    /// The value Copy, Paste and the memory keys use: the input's, else the
    /// last result.
    pub fn current_value(&self, ctx: &CalcContext) -> Option<Value> {
        match self.preview(ctx) {
            Preview::Ok(value) => Some(value),
            _ => self.history.last().map(|e| e.value),
        }
    }

    /// Enter: evaluate the input; on success it joins the history and
    /// becomes `ans`, and the input clears.
    pub fn submit(&mut self, ctx: &CalcContext) -> Option<Submitted> {
        let trimmed = self.input.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        let Ok(value) = calc::evaluate(&trimmed, &mut ctx.clone()) else {
            self.failed_input = Some(self.input.clone());
            return None;
        };
        let expr = if continues_ans(&trimmed) {
            format!("ans {trimmed}")
        } else {
            trimmed.clone()
        };
        let assign = assignment_target(&trimmed).map(str::to_string);
        self.history.push(Entry { expr, value });
        self.ans = Some(value);
        self.input.clear();
        self.hist_idx = None;
        self.new_var_hint = false;
        Some(Submitted { value, assign })
    }

    /// Keep an assigned variable where it belongs. A constant of that name is
    /// updated; a point lives for the session. Returns the number the drawing
    /// should store, if any.
    pub fn store(&mut self, name: &str, value: Value) -> Option<f64> {
        match value {
            Value::Number(n) => {
                if let Some(slot) = self.constants.iter_mut().find(|(c, _)| c == name) {
                    slot.1 = n;
                    return None;
                }
                self.session.remove(name);
                Some(n)
            }
            Value::Point(_) => {
                self.session.insert(name.to_string(), value);
                None
            }
        }
    }

    pub fn append(&mut self, text: &str) {
        self.input.push_str(text);
        self.hist_idx = None;
    }

    /// The literal a pick or a variable adds to the input.
    pub fn insert_value(&mut self, value: &Value) {
        let text = value.display();
        self.append(&text);
    }

    pub fn history_step(&mut self, up: bool) {
        if self.history.is_empty() {
            return;
        }
        match (up, self.hist_idx) {
            (true, None) => self.hist_idx = Some(self.history.len() - 1),
            (true, Some(i)) => self.hist_idx = Some(i.saturating_sub(1)),
            (false, None) => return,
            (false, Some(i)) if i + 1 >= self.history.len() => {
                self.hist_idx = None;
                self.input.clear();
                return;
            }
            (false, Some(i)) => self.hist_idx = Some(i + 1),
        }
        if let Some(i) = self.hist_idx {
            self.input = self.history[i].expr.clone();
        }
    }

    /// `input` as one operand: bare when it is a single name or number,
    /// otherwise in brackets; `ans` when it is empty and there is one.
    fn operand(&self) -> Option<String> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            return self.ans.map(|_| "ans".to_string());
        }
        let simple = trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
        Some(if simple {
            trimmed.to_string()
        } else {
            format!("({trimmed})")
        })
    }

    fn wrap(&mut self, build: impl Fn(&str) -> String, partial: &str) {
        match self.operand() {
            Some(o) => self.input = build(&o),
            None => self.input.push_str(partial),
        }
        self.hist_idx = None;
    }

    /// A key press that needs nothing from the app (all but `Eval`).
    /// Returns a footer status, if the key reports one.
    pub fn press(&mut self, key: Key, ctx: &CalcContext) -> Option<String> {
        match key {
            Key::Text(text) => self.append(text),
            Key::Back => {
                self.input.pop();
                self.hist_idx = None;
            }
            Key::Clear => {
                self.input.clear();
                self.hist_idx = None;
            }
            Key::Eval => {}
            Key::Sqrt => {
                let inner = match self.input.trim() {
                    "" => self.ans.map(|_| "ans".to_string()),
                    t => Some(t.to_string()),
                };
                match inner {
                    Some(inner) => self.input = format!("sqrt({inner})"),
                    None => self.input.push_str("sqrt("),
                }
                self.hist_idx = None;
            }
            Key::Inverse => self.wrap(|o| format!("1/{o}"), "1/"),
            Key::Square => self.wrap(|o| format!("{o}^2"), "^2"),
            Key::Cube => self.wrap(|o| format!("{o}^3"), "^3"),
            Key::MemStore | Key::MemAdd => {
                let n = match self.current_value(ctx) {
                    None => return Some(crate::t!("Nothing to store yet").into_owned()),
                    Some(Value::Point(_)) => {
                        return Some(crate::t!("Memory holds numbers only").into_owned())
                    }
                    Some(Value::Number(n)) => n,
                };
                if key == Key::MemStore {
                    self.memory = Some(n);
                    let shown = Value::Number(n).display();
                    return Some(crate::tf!("Stored {} in memory", shown).into_owned());
                }
                let total = self.memory.unwrap_or(0.0) + n;
                self.memory = Some(total);
                let shown = Value::Number(total).display();
                return Some(crate::tf!("Memory is now {}", shown).into_owned());
            }
            Key::MemRecall => match self.memory {
                Some(n) => self.insert_value(&Value::Number(n)),
                None => return Some(crate::t!("Memory is empty").into_owned()),
            },
            Key::MemClear => {
                self.memory = None;
                return Some(crate::t!("Memory cleared").into_owned());
            }
        }
        None
    }

    pub fn toggle(&mut self, section: Section) {
        self.open[section.index()] ^= true;
    }

    pub fn set_unit_cat(&mut self, cat: UnitCat) {
        self.unit_cat = cat;
        (self.unit_from, self.unit_to) = cat.defaults();
    }

    /// The converter's result, when its value reads as a number.
    pub fn converted(&self) -> Option<f64> {
        let value: f64 = self.unit_value.trim().replace(',', ".").parse().ok()?;
        let from = self.unit_cat.size(self.unit_from)?;
        let to = self.unit_cat.size(self.unit_to)?;
        let result = value * from / to;
        result.is_finite().then_some(result)
    }

    /// What "Insert" adds: the converted value with its unit suffix when the
    /// calculator reads that unit (lengths and angles), else the bare number.
    pub fn converted_literal(&self) -> Option<String> {
        let shown = Value::Number(self.converted()?).display();
        Some(match self.unit_cat {
            UnitCat::Area => shown,
            _ => format!("{shown}{}", self.unit_to),
        })
    }

    pub fn set_flash(&mut self, message: String) {
        self.flash = Some((message, iced::time::Instant::now()));
    }

    /// Drop the footer status once it has shown long enough.
    pub fn expire_flash(&mut self) {
        if self
            .flash
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed().as_secs_f32() >= FLASH_SECS - 0.05)
        {
            self.flash = None;
        }
    }

    pub fn flash_interval() -> std::time::Duration {
        std::time::Duration::from_secs_f32(FLASH_SECS)
    }
}

// ── View ────────────────────────────────────────────────────────────────

mod view;
pub use view::{view, Inputs, ICON};

#[cfg(test)]
mod tests;
