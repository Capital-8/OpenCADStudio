//! Built-in functions. Names are case-insensitive; trig follows the angle mode.

use std::f64::consts::TAU;
use std::ops::Range;

use glam::DVec3;

use super::eval::{finite, number};
use super::parser::Expr;
use super::value::{CalcContext, Value};
use super::CalcError;

/// Every built-in function name. These, `pi`, `ans` and the unit suffixes
/// cannot be used as variable names.
pub(super) const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sqrt", "abs", "round", "floor", "ceil",
    "ln", "log", "exp", "pow", "min", "max", "d2r", "r2d", "dist", "ang", "mid", "xof", "yof",
    "zof",
];

pub(super) fn call(
    name: &str,
    values: &[Value],
    args: &[Expr],
    span: Range<usize>,
    ctx: &CalcContext,
) -> Result<Value, CalcError> {
    let lower = name.to_ascii_lowercase();
    let mode = ctx.angle_mode;
    let arity = |range: std::ops::RangeInclusive<usize>, shown: &str| {
        if range.contains(&values.len()) {
            Ok(())
        } else {
            Err(CalcError::new(
                span.clone(),
                crate::tf!(
                    "{}() expects {} argument(s), found {}",
                    lower,
                    shown,
                    values.len()
                ),
            ))
        }
    };
    let num = |i: usize| number(values[i], args[i].span.clone());
    let point = |i: usize| match values[i] {
        Value::Point(p) => Ok(p),
        Value::Number(_) => Err(CalcError::new(
            args[i].span.clone(),
            crate::t!("expected a point, found a number"),
        )),
    };
    let unary = |f: fn(f64) -> f64| -> Result<Value, CalcError> {
        arity(1..=1, "1")?;
        Ok(Value::Number(f(num(0)?)))
    };

    let value = match lower.as_str() {
        "sin" | "cos" | "tan" => {
            arity(1..=1, "1")?;
            let a = mode.to_radians(num(0)?);
            Value::Number(match lower.as_str() {
                "sin" => a.sin(),
                "cos" => a.cos(),
                _ => a.tan(),
            })
        }
        "asin" | "acos" | "atan" => {
            arity(1..=1, "1")?;
            let x = num(0)?;
            let a = match lower.as_str() {
                "asin" => x.asin(),
                "acos" => x.acos(),
                _ => x.atan(),
            };
            Value::Number(mode.radians_to_mode(a))
        }
        "atan2" => {
            arity(2..=2, "2")?;
            Value::Number(mode.radians_to_mode(num(0)?.atan2(num(1)?)))
        }
        "sqrt" => unary(f64::sqrt)?,
        "abs" => unary(f64::abs)?,
        "floor" => unary(f64::floor)?,
        "ceil" => unary(f64::ceil)?,
        "ln" => unary(f64::ln)?,
        "log" => unary(f64::log10)?,
        "exp" => unary(f64::exp)?,
        "d2r" => unary(f64::to_radians)?,
        "r2d" => unary(f64::to_degrees)?,
        "round" => {
            arity(1..=2, "1-2")?;
            let x = num(0)?;
            if values.len() == 1 {
                Value::Number(x.round())
            } else {
                let places = num(1)?.round().clamp(-15.0, 15.0);
                let scale = 10f64.powf(places);
                Value::Number((x * scale).round() / scale)
            }
        }
        "pow" => {
            arity(2..=2, "2")?;
            Value::Number(num(0)?.powf(num(1)?))
        }
        "min" | "max" => {
            arity(1..=usize::MAX, "1+")?;
            let mut best = num(0)?;
            for i in 1..values.len() {
                let x = num(i)?;
                best = if lower == "min" { best.min(x) } else { best.max(x) };
            }
            Value::Number(best)
        }
        "dist" => {
            arity(2..=2, "2")?;
            Value::Number(point(0)?.distance(point(1)?))
        }
        "ang" => {
            // ang(v): direction of a vector; ang(p1,p2): from p1 towards p2.
            // Measured in the XY plane, counter-clockwise from +X.
            arity(1..=2, "1-2")?;
            let v: DVec3 = if values.len() == 1 {
                point(0)?
            } else {
                point(1)? - point(0)?
            };
            Value::Number(mode.radians_to_mode(v.y.atan2(v.x).rem_euclid(TAU)))
        }
        "mid" => {
            arity(2..=2, "2")?;
            Value::Point(point(0)?.midpoint(point(1)?))
        }
        "xof" | "yof" | "zof" => {
            arity(1..=1, "1")?;
            let p = point(0)?;
            Value::Number(match lower.as_str() {
                "xof" => p.x,
                "yof" => p.y,
                _ => p.z,
            })
        }
        _ => {
            return Err(CalcError::new(
                span,
                crate::tf!("unknown function '{}'", name),
            ))
        }
    };
    finite(value, span)
}
