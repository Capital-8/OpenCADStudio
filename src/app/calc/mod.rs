//! The calculator engine behind CAL and QuickCalc: arithmetic, points,
//! variables, `ans`, unit suffixes and a small function library.
//!
//! Self-contained on purpose — no UI and no scene access, so every surface
//! that evaluates typed input can share it.

mod eval;
mod functions;
mod lexer;
mod parser;
mod units;
mod value;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::ops::Range;

pub use value::{AngleMode, CalcContext, Value};

/// Longer input is refused outright. It keeps the syntax tree (whose depth
/// a long `1+1+…` chain grows) far from the stack limit; nobody types an
/// expression this long.
const MAX_INPUT_LEN: usize = 1024;

/// What went wrong, and where: `span` is the byte range of the offending
/// text in the input, for underlining.
#[derive(Debug, Clone, PartialEq)]
pub struct CalcError {
    pub span: Range<usize>,
    pub message: String,
}

impl CalcError {
    fn new(span: Range<usize>, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Evaluate one line: an expression, or `name = expression`. On success the
/// result becomes `ans`, and an assignment stores the variable.
pub fn evaluate(input: &str, ctx: &mut CalcContext) -> Result<Value, CalcError> {
    if input.len() > MAX_INPUT_LEN {
        return Err(CalcError::new(
            0..input.len(),
            crate::t!("expression is too long"),
        ));
    }
    let input = decimal_comma(input);
    let tokens = lexer::tokenize(&input)?;
    let (target, expr) = match parser::parse(&tokens, input.len())? {
        parser::Statement::Expr(expr) => (None, expr),
        parser::Statement::Assign { name, value, .. } => (Some(name), value),
    };
    let value = eval::eval(&expr, ctx)?;
    if let Some(name) = target {
        ctx.vars.insert(name, value);
    }
    ctx.ans = Some(value);
    Ok(value)
}

/// Names a variable cannot take: functions, `pi`, `e`, `ans`, `mod` and
/// unit suffixes.
pub fn is_reserved_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    functions::FUNCTIONS.contains(&lower.as_str())
        || lower == "pi"
        || lower == "ans"
        || lower == "e"
        || lower == "mod"
        || units::is_unit_name(&lower)
}

/// A lone `3,5` is 3.5, as at any value prompt — commas otherwise separate
/// coordinates and arguments. Same length, so error spans still line up.
fn decimal_comma(input: &str) -> Cow<'_, str> {
    let t = input.trim();
    let digits = t.strip_prefix(['-', '+']).unwrap_or(t);
    match digits.split_once(',') {
        Some((a, b))
            if !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit()) =>
        {
            Cow::Owned(input.replacen(',', ".", 1))
        }
        _ => Cow::Borrowed(input),
    }
}
