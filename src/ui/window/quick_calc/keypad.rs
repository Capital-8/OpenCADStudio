//! The keypad's keys and memory. Unary keys work the way a desk calculator's
//! do: on the last number typed, not on the whole input.

use super::QuickCalcPalette;
use crate::app::calc::{CalcContext, Value};

/// Standard or Scientific.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeypadMode {
    #[default]
    Standard,
    Scientific,
}

/// The menus that open over the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flyout {
    Memory,
    Trigonometry,
    Function,
    Point,
}

/// A keypad key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Appended as written: digits, `.`, brackets, `pi`, `e`, operators with
    /// their spaces, and the point menu's `[ , ]` and `dist(`.
    Text(&'static str),
    /// ⌫: one character.
    Back,
    /// CE: the last operand, or the trailing operator when there is none.
    ClearEntry,
    /// C: the whole input.
    Clear,
    /// =: evaluate, as Enter.
    Eval,
    /// Written after the operand: x² (`^2`), x³ (`^3`), % (`/100`).
    Postfix(&'static str),
    /// Written before the operand: ¹⁄x (`1/`), 10ˣ (`10^`), 2ˣ (`2^`).
    Prefix(&'static str),
    /// The operand as the argument: `sqrt`, `abs`, `fact`, `log`, ….
    Func(&'static str),
    /// A trigonometric function by its base name (`sin` … `cot`); 2nd makes
    /// it the inverse and hyp, where `hyp` allows, the hyperbolic one.
    Trig { name: &'static str, hyp: bool },
    /// The operand as the first of two arguments: `root(op, ` / `logb(op, `.
    TwoArg(&'static str),
    /// +/−: `(-op)`, and back.
    Negate,
    /// 2nd: the second functions, until pressed again.
    Second,
    /// hyp: hyperbolic trigonometry, until pressed again.
    Hyp,
}

impl Key {
    /// Keys that pick something from a flyout close it; the toggles keep it
    /// open.
    pub fn closes_flyout(self) -> bool {
        !matches!(self, Key::Second | Key::Hyp)
    }
}

/// The memory row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOp {
    Clear,
    Recall,
    Add,
    Subtract,
    Store,
}

/// Where the last operand of `input` starts: the trailing number, name,
/// number with a unit (`1.2m`), bracketed group, point, or function call.
/// `None` when the input ends in an operator (or is empty).
pub fn split_operand(input: &str) -> Option<usize> {
    let t = input.trim_end().as_bytes();
    let last = *t.last()?;
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.';
    let mut i = t.len() - 1;
    if last == b')' || last == b']' {
        let mut depth = 0i32;
        loop {
            match t[i] {
                b')' | b']' => depth += 1,
                b'(' | b'[' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            if i == 0 {
                return None;
            }
            i -= 1;
        }
        // A call: the name before its bracket belongs to the operand.
        if t[i] == b'(' {
            while i > 0 && (t[i - 1].is_ascii_alphanumeric() || t[i - 1] == b'_') {
                i -= 1;
            }
        }
        Some(i)
    } else if word(last) {
        while i > 0 && word(t[i - 1]) {
            i -= 1;
        }
        // `mod` is an operator, not a name.
        (!t[i..].eq_ignore_ascii_case(b"mod")).then_some(i)
    } else {
        None
    }
}

/// Whether `o` is one bracketed group, `( … )`, from end to end.
fn wrapped(o: &str) -> bool {
    if o.len() < 2 || !o.starts_with('(') || !o.ends_with(')') {
        return false;
    }
    let mut depth = 0;
    for (i, c) in o.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && i < o.len() - 1 {
            return false;
        }
    }
    true
}

/// Whether `o` reads as one value without brackets around it.
fn atomic(o: &str) -> bool {
    let name_end = o
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(o.len());
    o.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        || (name_end > 0 && wrapped(&o[name_end..]))
        || (o.starts_with('[') && o.ends_with(']'))
        || wrapped(o)
}

fn paren(o: &str) -> String {
    if atomic(o) {
        o.to_string()
    } else {
        format!("({o})")
    }
}

/// Drop one bracket pair that a function call's own brackets make redundant.
fn strip(o: &str) -> &str {
    if wrapped(o) {
        &o[1..o.len() - 1]
    } else {
        o
    }
}

/// The input as `head` + last operand, or `None` when it has no operand.
fn split(input: &str) -> Option<(&str, &str)> {
    let t = input.trim_end();
    split_operand(t).map(|i| t.split_at(i))
}

impl QuickCalcPalette {
    /// Rewrite the last operand with `build`. Without one, an empty input
    /// uses `ans`; otherwise `fallback` is typed, or the key needs a number.
    fn on_operand(
        &mut self,
        build: impl Fn(&str) -> String,
        fallback: Option<&str>,
    ) -> Option<String> {
        if let Some((head, op)) = split(&self.input) {
            self.input = format!("{head}{}", build(op));
        } else if self.input.trim().is_empty() && self.ans.is_some() {
            self.input = build("ans");
        } else if let Some(text) = fallback {
            self.input.push_str(text);
        } else {
            return Some(crate::t!("Type a number first").into_owned());
        }
        self.hist_idx = None;
        None
    }

    /// The trigonometric function a key stands for now, from its base name
    /// and the 2nd and hyp toggles.
    pub fn trig_name(&self, name: &str, hyp: bool) -> String {
        let inverse = if self.second { "a" } else { "" };
        let hyperbolic = if hyp && self.hyp { "h" } else { "" };
        format!("{inverse}{name}{hyperbolic}")
    }

    /// A key that needs nothing from the app (all but `Eval`). Returns a
    /// footer status when the key has something to say.
    pub fn press(&mut self, key: Key) -> Option<String> {
        if key.closes_flyout() && self.flyout != Some(Flyout::Memory) {
            self.flyout = None;
        }
        let call = |name: &str| {
            let name = name.to_string();
            move |o: &str| format!("{name}({})", strip(o))
        };
        match key {
            Key::Text(text) => self.append(text),
            Key::Back => {
                let kept = self.input.trim_end();
                let mut kept =
                    kept[..kept.len() - kept.chars().last().map_or(0, char::len_utf8)].to_string();
                // One space stays between an operator and what comes next.
                if kept.ends_with(char::is_whitespace) {
                    kept.truncate(kept.trim_end().len());
                    kept.push(' ');
                }
                self.input = kept;
                self.hist_idx = None;
            }
            Key::ClearEntry => {
                match split(&self.input) {
                    Some((head, _)) => self.input = head.to_string(),
                    None => {
                        let mut t = self.input.trim_end().to_string();
                        loop {
                            let trimmed = t.trim_end();
                            let cut = if trimmed.to_ascii_lowercase().ends_with("mod") {
                                3
                            } else if trimmed.ends_with(['+', '-', '*', '/', '^', ',', '(', '[']) {
                                1
                            } else {
                                break;
                            };
                            t = trimmed[..trimmed.len() - cut].to_string();
                        }
                        self.input = t.trim_end().to_string();
                        if !self.input.is_empty() {
                            self.input.push(' ');
                        }
                    }
                }
                self.hist_idx = None;
            }
            Key::Clear => {
                self.input.clear();
                self.hist_idx = None;
            }
            Key::Eval => {}
            Key::Postfix(suffix) => return self.on_operand(|o| paren(o) + suffix, None),
            Key::Prefix(prefix) => {
                return self.on_operand(|o| format!("{prefix}{}", paren(o)), Some(prefix))
            }
            Key::Func(name) => return self.on_operand(call(name), Some(&format!("{name}("))),
            Key::Trig { name, hyp } => {
                let name = self.trig_name(name, hyp);
                return self.on_operand(call(&name), Some(&format!("{name}(")));
            }
            Key::TwoArg(name) => {
                return self.on_operand(
                    |o| format!("{name}({}, ", strip(o)),
                    Some(&format!("{name}(")),
                )
            }
            Key::Negate => {
                return self.on_operand(
                    |o| match o.strip_prefix("(-").filter(|_| wrapped(o)) {
                        Some(inner) => inner[..inner.len() - 1].to_string(),
                        None => format!("(-{o})"),
                    },
                    Some("(-"),
                )
            }
            Key::Second => self.second ^= true,
            Key::Hyp => self.hyp ^= true,
        }
        None
    }

    /// The memory row. Memory holds numbers, newest first.
    pub fn memory_op(&mut self, op: MemoryOp, ctx: &CalcContext) -> Option<String> {
        let number = || match self.current_value(ctx) {
            None => Err(crate::t!("Nothing to store yet").into_owned()),
            Some(Value::Point(_)) => Err(crate::t!("Memory holds numbers only").into_owned()),
            Some(Value::Number(n)) => Ok(n),
        };
        match op {
            MemoryOp::Clear => {
                self.memory.clear();
                if self.flyout == Some(Flyout::Memory) {
                    self.flyout = None;
                }
                None
            }
            MemoryOp::Recall => {
                if let Some(top) = self.memory.first().copied() {
                    self.insert_value(&Value::Number(top));
                }
                None
            }
            MemoryOp::Store => match number() {
                Ok(n) => {
                    self.memory.insert(0, n);
                    None
                }
                Err(message) => Some(message),
            },
            MemoryOp::Add | MemoryOp::Subtract => match number() {
                Ok(n) => {
                    let n = if op == MemoryOp::Add { n } else { -n };
                    match self.memory.first_mut() {
                        Some(top) => *top += n,
                        None => self.memory.push(n),
                    }
                    None
                }
                Err(message) => Some(message),
            },
        }
    }

    /// A value picked from the memory flyout goes into the input.
    pub fn memory_insert(&mut self, index: usize) {
        if let Some(n) = self.memory.get(index).copied() {
            self.insert_value(&Value::Number(n));
        }
        self.flyout = None;
    }

    pub fn toggle_flyout(&mut self, flyout: Flyout) {
        self.flyout = if self.flyout == Some(flyout) {
            None
        } else {
            Some(flyout)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(input: &str) -> Option<&str> {
        let t = input.trim_end();
        split_operand(t).map(|i| &t[i..])
    }

    #[test]
    fn splits_the_last_operand() {
        assert_eq!(op("12"), Some("12"));
        assert_eq!(op("3 + 12.5"), Some("12.5"));
        assert_eq!(op("3 + 1.2m"), Some("1.2m"));
        assert_eq!(op("wall * ans"), Some("ans"));
        assert_eq!(op("2 * (3 + (4 - 1))"), Some("(3 + (4 - 1))"));
        assert_eq!(op("1 + [3, 4]"), Some("[3, 4]"));
        assert_eq!(op("1 + dist([0,0], [3,4])"), Some("dist([0,0], [3,4])"));
        assert_eq!(op("2 + sqrt(9)  "), Some("sqrt(9)"));
        assert_eq!(op("2 + "), None);
        assert_eq!(op("2 *"), None);
        assert_eq!(op("7 mod "), None);
        assert_eq!(op("7 mod"), None);
        assert_eq!(op(""), None);
        assert_eq!(op("1 + 2)"), None);
    }

    fn after(input: &str, key: Key) -> String {
        let mut p = QuickCalcPalette {
            input: input.into(),
            ..Default::default()
        };
        p.press(key);
        p.input
    }

    #[test]
    fn unary_keys_act_on_the_last_operand() {
        assert_eq!(after("3 + 4", Key::Postfix("^2")), "3 + 4^2");
        assert_eq!(after("3 + (1 + 1)", Key::Postfix("^3")), "3 + (1 + 1)^3");
        assert_eq!(after("50", Key::Postfix("/100")), "50/100");
        assert_eq!(after("3 + 4", Key::Prefix("1/")), "3 + 1/4");
        assert_eq!(after("2 * wall", Key::Prefix("10^")), "2 * 10^wall");
        assert_eq!(after("9", Key::Func("sqrt")), "sqrt(9)");
        assert_eq!(after("1 + (2 + 7)", Key::Func("sqrt")), "1 + sqrt(2 + 7)");
        assert_eq!(after("1.2m", Key::Func("abs")), "abs(1.2m)");
        assert_eq!(after("5", Key::Func("fact")), "fact(5)");
        assert_eq!(after("27", Key::TwoArg("root")), "root(27, ");
        assert_eq!(after("8", Key::TwoArg("logb")), "logb(8, ");
        assert_eq!(after("[1,2]", Key::Func("xof")), "xof([1,2])");
        // A call is atomic; a sum is bracketed.
        assert_eq!(after("sqrt(9)", Key::Postfix("^2")), "sqrt(9)^2");
        assert_eq!(after("(-3)", Key::Postfix("^2")), "(-3)^2");
    }

    #[test]
    fn negate_toggles() {
        assert_eq!(after("3 + 4", Key::Negate), "3 + (-4)");
        assert_eq!(after("3 + (-4)", Key::Negate), "3 + 4");
        assert_eq!(after("(1 + 2)", Key::Negate), "(-(1 + 2))");
        assert_eq!(after("", Key::Negate), "(-");
    }

    #[test]
    fn no_operand() {
        let mut p = QuickCalcPalette {
            input: "2 + ".into(),
            ..Default::default()
        };
        assert!(p.press(Key::Postfix("^2")).is_some());
        assert_eq!(p.input, "2 + ");
        assert!(p.press(Key::Func("sin")).is_none());
        assert_eq!(p.input, "2 + sin(");
        // Empty with a previous result: it applies to ans.
        let mut p = QuickCalcPalette {
            ans: Some(Value::Number(4.0)),
            ..Default::default()
        };
        p.press(Key::Postfix("^2"));
        assert_eq!(p.input, "ans^2");
        // Empty without one: postfix keys need a number.
        let mut p = QuickCalcPalette::default();
        assert!(p.press(Key::Postfix("/100")).is_some());
        assert!(p.input.is_empty());
    }

    #[test]
    fn clear_entry_and_backspace() {
        assert_eq!(after("12 + 34", Key::ClearEntry), "12 + ");
        assert_eq!(after("12 + ", Key::ClearEntry), "12 ");
        assert_eq!(after("7 mod ", Key::ClearEntry), "7 ");
        assert_eq!(after("12", Key::ClearEntry), "");
        assert_eq!(after("12 + 34", Key::Clear), "");
        assert_eq!(after("123", Key::Back), "12");
        assert_eq!(after("12 + ", Key::Back), "12 ");
        assert_eq!(after("", Key::Back), "");
        assert_eq!(after("2π", Key::Back), "2");
    }

    #[test]
    fn trig_follows_second_and_hyp() {
        let mut p = QuickCalcPalette {
            input: "30".into(),
            ..Default::default()
        };
        p.press(Key::Second);
        p.press(Key::Hyp);
        assert_eq!(p.trig_name("sin", true), "asinh");
        assert_eq!(p.trig_name("sec", false), "asec");
        p.press(Key::Trig {
            name: "sin",
            hyp: true,
        });
        assert_eq!(p.input, "asinh(30)");
        // 2nd stays on until pressed again.
        assert!(p.second);
        p.press(Key::Second);
        p.press(Key::Hyp);
        assert_eq!(p.trig_name("cos", true), "cos");
    }

    #[test]
    fn flyouts_close_on_a_pick_but_not_on_toggles() {
        let mut p = QuickCalcPalette::default();
        p.toggle_flyout(Flyout::Trigonometry);
        p.press(Key::Second);
        assert_eq!(p.flyout, Some(Flyout::Trigonometry));
        p.press(Key::Trig {
            name: "tan",
            hyp: true,
        });
        assert_eq!(p.flyout, None);
        p.toggle_flyout(Flyout::Point);
        p.toggle_flyout(Flyout::Point);
        assert_eq!(p.flyout, None);
    }

    #[test]
    fn memory_list() {
        let ctx = CalcContext::default();
        let mut p = QuickCalcPalette::default();
        assert!(p.memory_op(MemoryOp::Store, &ctx).is_some());
        p.input = "870".into();
        p.memory_op(MemoryOp::Store, &ctx);
        p.input = "30".into();
        p.memory_op(MemoryOp::Store, &ctx);
        assert_eq!(p.memory, vec![30.0, 870.0]);
        p.input = "5".into();
        p.memory_op(MemoryOp::Add, &ctx);
        p.memory_op(MemoryOp::Subtract, &ctx);
        p.memory_op(MemoryOp::Subtract, &ctx);
        assert_eq!(p.memory, vec![25.0, 870.0]);
        p.input.clear();
        p.memory_op(MemoryOp::Recall, &ctx);
        assert_eq!(p.input, "25");
        p.input.clear();
        p.toggle_flyout(Flyout::Memory);
        p.memory_insert(1);
        assert_eq!(p.input, "870");
        assert_eq!(p.flyout, None);
        p.input = "[1,2]".into();
        assert!(p.memory_op(MemoryOp::Store, &ctx).is_some());
        p.memory_op(MemoryOp::Clear, &ctx);
        assert!(p.memory.is_empty());
        // M+ on an empty memory stores the value.
        p.input = "4".into();
        p.memory_op(MemoryOp::Subtract, &ctx);
        assert_eq!(p.memory, vec![-4.0]);
    }
}
