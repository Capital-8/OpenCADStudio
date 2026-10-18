//! Calculator engine tests.

use super::value::AngleMode;
use super::*;
use glam::DVec3;

fn ctx() -> CalcContext {
    CalcContext::default()
}

fn num(input: &str) -> f64 {
    num_in(input, &mut ctx())
}

fn num_in(input: &str, ctx: &mut CalcContext) -> f64 {
    match evaluate(input, ctx) {
        Ok(Value::Number(n)) => n,
        other => panic!("{input:?} → {other:?}"),
    }
}

fn point(input: &str) -> DVec3 {
    match evaluate(input, &mut ctx()) {
        Ok(Value::Point(p)) => p,
        other => panic!("{input:?} → {other:?}"),
    }
}

fn err(input: &str) -> CalcError {
    err_in(input, &mut ctx())
}

fn err_in(input: &str, ctx: &mut CalcContext) -> CalcError {
    match evaluate(input, ctx) {
        Err(e) => e,
        ok => panic!("{input:?} should fail, got {ok:?}"),
    }
}

/// The text an error underlines.
fn underlined(input: &str) -> String {
    input[err(input).span].to_string()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9 * b.abs().max(1.0)
}

macro_rules! assert_close {
    ($a:expr, $b:expr) => {{
        let (a, b) = ($a, $b);
        assert!(close(a, b), "{a} != {b}");
    }};
}

// ── Arithmetic ──────────────────────────────────────────────────────────

#[test]
fn precedence() {
    assert_eq!(num("2+3*4"), 14.0);
    assert_eq!(num("(2+3)*4"), 20.0);
    assert_eq!(num("10-4-3"), 3.0);
    assert_eq!(num("100/10/5"), 2.0);
    assert_eq!(num("2^3^2"), 512.0);
    assert_eq!(num("2*3^2"), 18.0);
    assert_eq!(num(" 1 +  2 "), 3.0);
}

#[test]
fn unary_minus() {
    assert_eq!(num("-5"), -5.0);
    assert_eq!(num("--5"), 5.0);
    assert_eq!(num("+-+5"), -5.0);
    assert_eq!(num("-2^2"), -4.0);
    assert_eq!(num("2^-1"), 0.5);
    assert_eq!(num("3*-2"), -6.0);
    assert_eq!(num("-(2+3)"), -5.0);
}

#[test]
fn numbers() {
    assert_eq!(num(".5"), 0.5);
    assert_eq!(num("1e3"), 1000.0);
    assert_eq!(num("2.5E-1"), 0.25);
    assert_eq!(underlined("1.2.3"), "1.2.3");
}

#[test]
fn lone_decimal_comma_reads_as_a_point() {
    assert_eq!(num("3,5"), 3.5);
    assert_eq!(num(" -3,25 "), -3.25);
    // Anywhere else the comma separates.
    assert_eq!(point("[3,5]"), DVec3::new(3.0, 5.0, 0.0));
    assert_eq!(num("max(3,5)"), 5.0);
    assert!(evaluate("3,5,7", &mut ctx()).is_err());
}

#[test]
fn display_drops_float_noise() {
    let show = |s: &str| evaluate(s, &mut ctx()).unwrap().display();
    assert_eq!(show("0.1+0.2"), "0.3");
    assert_eq!(show("1/3"), "0.3333333333");
    assert_eq!(show("-0"), "0");
    assert_eq!(show("1e-9"), "1e-9");
    assert_eq!(show("[1,2]"), "[1,2]");
    assert_eq!(show("[1,2,3.5]"), "[1,2,3.5]");
}

// ── Points ──────────────────────────────────────────────────────────────

#[test]
fn point_literals_and_arithmetic() {
    assert_eq!(point("[1,2]"), DVec3::new(1.0, 2.0, 0.0));
    assert_eq!(point("[1,2,3]"), DVec3::new(1.0, 2.0, 3.0));
    assert_eq!(point("[1+1, 2*3]"), DVec3::new(2.0, 6.0, 0.0));
    assert_eq!(point("[1,2]+[3,4,5]"), DVec3::new(4.0, 6.0, 5.0));
    assert_eq!(point("[5,5]-[1,2]"), DVec3::new(4.0, 3.0, 0.0));
    assert_eq!(point("[1,2]*2"), DVec3::new(2.0, 4.0, 0.0));
    assert_eq!(point("2*[1,2]"), DVec3::new(2.0, 4.0, 0.0));
    assert_eq!(point("[2,4]/2"), DVec3::new(1.0, 2.0, 0.0));
    assert_eq!(point("-[1,2]"), DVec3::new(-1.0, -2.0, 0.0));
}

#[test]
fn point_type_errors() {
    assert_eq!(underlined("1 + [1,2]"), "1 + [1,2]");
    assert_eq!(underlined("[1,2]*[3,4]"), "[1,2]*[3,4]");
    assert_eq!(underlined("2/[1,2]"), "[1,2]");
    assert_eq!(underlined("[1,2]^2"), "[1,2]");
    assert_eq!(underlined("[[1,2],3]"), "[1,2]");
    assert_eq!(underlined("[1]"), "[1]");
    assert_eq!(underlined("[1,2,3,4]"), "[1,2,3,4]");
    assert_eq!(underlined("sqrt([1,2])"), "[1,2]");
}

// ── Variables and ans ───────────────────────────────────────────────────

#[test]
fn assignment_stores_and_returns() {
    let mut c = ctx();
    assert_eq!(num_in("wall = 3600", &mut c), 3600.0);
    assert_eq!(num_in("wall + 90", &mut c), 3690.0);
    assert_eq!(num_in("wall = wall * 2", &mut c), 7200.0);
    assert_eq!(c.vars["wall"], Value::Number(7200.0));
    num_in("p = [1,2]", &mut c);
    assert_eq!(c.vars["p"], Value::Point(DVec3::new(1.0, 2.0, 0.0)));
    // Case-sensitive names.
    assert_eq!(err_in("WALL", &mut c).message, "unknown variable 'WALL'");
}

#[test]
fn reserved_names_cannot_be_assigned() {
    for name in ["sin", "PI", "ans", "m", "deg", "dist"] {
        let input = format!("{name} = 1");
        let e = err(&input);
        assert_eq!(&input[e.span], name);
    }
}

#[test]
fn failed_evaluation_changes_nothing() {
    let mut c = ctx();
    num_in("a = 1", &mut c);
    err_in("a = 1/0", &mut c);
    assert_eq!(c.vars["a"], Value::Number(1.0));
    assert_eq!(c.ans, Some(Value::Number(1.0)));
}

#[test]
fn ans_holds_the_previous_result() {
    let mut c = ctx();
    assert_eq!(err_in("ans", &mut c).span, 0..3);
    num_in("870", &mut c);
    assert_eq!(num_in("ans * 2", &mut c), 1740.0);
    assert_eq!(num_in("ANS + 1", &mut c), 1741.0);
}

#[test]
fn leading_operator_continues_from_ans() {
    let mut c = ctx();
    assert_eq!(err_in("*2", &mut c).span, 0..1);
    num_in("10", &mut c);
    assert_eq!(num_in("*2", &mut c), 20.0);
    assert_eq!(num_in("/4", &mut c), 5.0);
    assert_eq!(num_in("^2", &mut c), 25.0);
    assert_eq!(num_in(" * 2 + 1", &mut c), 51.0);
    // `+`/`-` stay signs.
    assert_eq!(num_in("-1", &mut c), -1.0);
}

// ── Units ───────────────────────────────────────────────────────────────

#[test]
fn length_suffixes_convert_to_the_drawing_unit() {
    let mut mm = ctx();
    assert_close!(num_in("3.6m", &mut mm), 3600.0);
    assert_close!(num_in("3.6 m", &mut mm), 3600.0);
    assert_close!(num_in("12cm", &mut mm), 120.0);
    assert_close!(num_in("1km", &mut mm), 1_000_000.0);
    assert_close!(num_in("5mm", &mut mm), 5.0);
    assert_close!(num_in("1in", &mut mm), 25.4);
    assert_close!(num_in("1ft", &mut mm), 304.8);
    assert_close!(num_in("1M + 10MM", &mut mm), 1010.0);

    let mut metres = CalcContext::new(AngleMode::Degrees, 1000.0);
    assert_close!(num_in("1500mm", &mut metres), 1.5);
    assert_close!(num_in("2", &mut metres), 2.0);

    let mut inches = CalcContext::new(AngleMode::Degrees, 25.4);
    assert_close!(num_in("2'9\"", &mut inches), 33.0);
    assert_close!(num_in("3'", &mut inches), 36.0);
    assert_close!(num_in("9\"", &mut inches), 9.0);
    assert_close!(num_in("2'9\" + 3\"", &mut inches), 36.0);
}

#[test]
fn angle_suffixes_follow_the_angle_mode() {
    let mut deg = ctx();
    assert_close!(num_in("90deg", &mut deg), 90.0);
    assert_close!(num_in("1rad", &mut deg), 1f64.to_degrees());
    assert_close!(num_in("100grad", &mut deg), 90.0);
    let mut rad = CalcContext::new(AngleMode::Radians, 1.0);
    assert_close!(num_in("180deg", &mut rad), std::f64::consts::PI);
    assert_close!(num_in("200grad", &mut rad), std::f64::consts::PI);
    assert_close!(num_in("sin(30deg)", &mut rad), 0.5);
}

#[test]
fn unknown_unit_is_underlined() {
    assert_eq!(underlined("2x"), "x");
    assert_eq!(underlined("2 x"), "x");
}

// ── Functions ───────────────────────────────────────────────────────────

#[test]
fn trig_in_degrees() {
    assert_close!(num("sin(30)"), 0.5);
    assert_close!(num("cos(60)"), 0.5);
    assert_close!(num("tan(45)"), 1.0);
    assert_close!(num("asin(0.5)"), 30.0);
    assert_close!(num("acos(0.5)"), 60.0);
    assert_close!(num("atan(1)"), 45.0);
    assert_close!(num("atan2(1,-1)"), 135.0);
}

#[test]
fn trig_in_radians() {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let mut c = CalcContext::new(AngleMode::Radians, 1.0);
    assert_close!(num_in("sin(pi/2)", &mut c), 1.0);
    assert_close!(num_in("cos(pi)", &mut c), -1.0);
    assert_close!(num_in("tan(pi/4)", &mut c), 1.0);
    assert_close!(num_in("asin(1)", &mut c), FRAC_PI_2);
    assert_close!(num_in("acos(-1)", &mut c), PI);
    assert_close!(num_in("atan(1)", &mut c), FRAC_PI_4);
    assert_close!(num_in("atan2(1,1)", &mut c), FRAC_PI_4);
    assert_close!(num_in("ang([0,0],[0,1])", &mut c), FRAC_PI_2);
}

#[test]
fn numeric_functions() {
    assert_eq!(num("sqrt(16)"), 4.0);
    assert_eq!(num("abs(-3)"), 3.0);
    assert_eq!(num("round(2.5)"), 3.0);
    assert_eq!(num("round(3.14159, 2)"), 3.14);
    assert_eq!(num("floor(-1.5)"), -2.0);
    assert_eq!(num("ceil(1.2)"), 2.0);
    assert_close!(num("ln(exp(2))"), 2.0);
    assert_close!(num("log(1000)"), 3.0);
    assert_close!(num("exp(0)"), 1.0);
    assert_eq!(num("pow(2,10)"), 1024.0);
    assert_eq!(num("min(4,2,8)"), 2.0);
    assert_eq!(num("max(4,2,8)"), 8.0);
    assert_eq!(num("max(7)"), 7.0);
    assert_close!(num("d2r(180)"), std::f64::consts::PI);
    assert_close!(num("r2d(pi)"), 180.0);
    assert_close!(num("pi"), std::f64::consts::PI);
    assert_eq!(num("SQRT(9)"), 3.0);
}

#[test]
fn point_functions() {
    assert_eq!(num("dist([0,0],[3,4])"), 5.0);
    assert_eq!(num("dist([1,1,1],[1,1,4])"), 3.0);
    assert_close!(num("ang([1,1])"), 45.0);
    assert_close!(num("ang([0,0],[-1,0])"), 180.0);
    assert_close!(num("ang([0,0],[0,-1])"), 270.0);
    assert_eq!(point("mid([0,0],[4,2,6])"), DVec3::new(2.0, 1.0, 3.0));
    assert_eq!(num("xof([1,2,3])"), 1.0);
    assert_eq!(num("yof([1,2,3])"), 2.0);
    assert_eq!(num("zof([1,2,3])"), 3.0);
    assert_eq!(num("zof([1,2])"), 0.0);
    assert_eq!(underlined("xof(5)"), "5");
}

#[test]
fn every_function_is_reserved_and_callable() {
    for name in functions::FUNCTIONS {
        assert!(is_reserved_name(name), "{name}");
        // Calling with the wrong number of arguments is an error at the
        // call, never a panic.
        let input = format!("{name}()");
        let e = err(&input);
        assert_eq!(e.span, 0..input.len(), "{name}");
    }
}

#[test]
fn function_errors() {
    assert_eq!(underlined("foo(1)"), "foo(1)");
    assert_eq!(underlined("1 + sqrt(1, 2)"), "sqrt(1, 2)");
    assert_eq!(underlined("1 + sqrt(-1)"), "sqrt(-1)");
    assert_eq!(underlined("asin(2)"), "asin(2)");
    assert_eq!(underlined("ln(0)"), "ln(0)");
}

// ── Errors ──────────────────────────────────────────────────────────────

#[test]
fn error_spans() {
    assert_eq!(underlined("2 + x"), "x");
    assert_eq!(underlined("10 / (5-5)"), "(5-5)");
    assert_eq!(underlined("2 $ 3"), "$");
    assert_eq!(underlined("2 + * 3"), "*");
    assert_eq!(underlined("(2 + 3"), "(2 + 3");
    assert_eq!(underlined("[1, 2"), "[1, 2");
    assert_eq!(underlined("2 3"), "3");
    assert_eq!(underlined("2)"), ")");
    assert_eq!(err("2 +").span, 3..3);
    assert_eq!(err("").span, 0..0);
    assert_eq!(err("   ").span, 0..3);
    assert_eq!(underlined("1e308 * 10"), "1e308 * 10");
}

#[test]
fn spans_are_byte_offsets_after_unicode() {
    assert_eq!(underlined("é + 1"), "é");
    assert_eq!(underlined("1 + √4"), "√");
}

#[test]
fn deep_nesting_is_bounded() {
    let ok = format!("{}1{}", "(".repeat(40), ")".repeat(40));
    assert_eq!(num(&ok), 1.0);
    let deep = format!("{}1{}", "(".repeat(1000), ")".repeat(1000));
    assert_eq!(err(&deep).message, "expression is too deeply nested");
    let minus = format!("{}1", "-".repeat(1000));
    assert!(evaluate(&minus, &mut ctx()).is_err());
    // A long flat chain is fine; an absurdly long input is refused.
    let chain = vec!["1"; 500].join("+");
    assert_eq!(num(&chain), 500.0);
    let huge = vec!["1"; 1000].join("+");
    assert_eq!(err(&huge).span, 0..huge.len());
}

#[test]
fn random_input_never_panics() {
    const ALPHABET: &[&str] = &[
        "1", "2.5", "0", "e", "1e", "+", "-", "*", "/", "^", "(", ")", "[", "]", ",", "=", " ",
        "'", "\"", "pi", "ans", "x", "sin", "dist", "mid", "m", "deg", "√", ".", "é", "$",
    ];
    let mut seed: u64 = 0x5eed_cafe;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..20_000 {
        let len = next() % 24;
        let input: String = (0..len).map(|_| ALPHABET[next() % ALPHABET.len()]).collect();
        let mut c = ctx();
        c.vars.insert("x".into(), Value::Number(2.0));
        if let Err(e) = evaluate(&input, &mut c) {
            assert!(e.span.start <= e.span.end && e.span.end <= input.len(), "{input:?}");
            assert!(input.is_char_boundary(e.span.start), "{input:?}");
            assert!(input.is_char_boundary(e.span.end), "{input:?}");
            assert!(!e.message.is_empty());
        }
    }
}
