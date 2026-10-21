use super::*;
use glam::DVec3;

fn ctx(p: &QuickCalcPalette) -> CalcContext {
    p.context(&[("wall".into(), 300.0)], 1.0)
}

fn submit(p: &mut QuickCalcPalette, input: &str) -> Option<Submitted> {
    p.input = input.into();
    let c = ctx(p);
    p.submit(&c)
}

#[test]
fn enter_history_and_errors() {
    let mut p = QuickCalcPalette::default();
    assert_eq!(
        submit(&mut p, "2*wall + 3*90").unwrap().value,
        Value::Number(870.0)
    );
    // A leading operator continues from ans, and the history says so.
    submit(&mut p, "/ 2").unwrap();
    assert_eq!(p.history[1].expr, "ans / 2");
    assert_eq!(p.ans, Some(Value::Number(435.0)));
    assert!(p.input.is_empty());
    // An error shows only after Enter, and only until the input changes.
    p.input = "2 +".into();
    assert!(!p.error_shown());
    assert!(submit(&mut p, "2 +").is_none());
    assert!(p.error_shown());
    assert_eq!(p.history.len(), 2);
    p.append(" 3");
    assert!(!p.error_shown());
    // ↑ / ↓ walk the history and come back to an empty input.
    p.history_step(true);
    assert_eq!(p.input, "ans / 2");
    p.history_step(true);
    p.history_step(true);
    assert_eq!(p.input, "2*wall + 3*90");
    p.history_step(false);
    p.history_step(false);
    assert_eq!(p.input, "");
}

#[test]
fn variables_and_constants() {
    let mut p = QuickCalcPalette::default();
    p.constants.push(("wall".into(), 1.0));
    p.constants.push(("brick".into(), 210.0));
    // The drawing's variable wins over a constant of the same name.
    assert_eq!(ctx(&p).vars["wall"], Value::Number(300.0));
    let s = submit(&mut p, "height = 2800").unwrap();
    assert_eq!(s.assign.as_deref(), Some("height"));
    // A number goes to the drawing, a constant is updated in place, and a
    // point lives for the session.
    assert_eq!(p.store("height", s.value), Some(2800.0));
    assert_eq!(p.store("brick", Value::Number(215.0)), None);
    assert_eq!(p.constants[1], ("brick".into(), 215.0));
    let pt = Value::Point(DVec3::new(1.0, 2.0, 0.0));
    assert_eq!(p.store("p1", pt), None);
    assert_eq!(ctx(&p).vars["p1"], pt);
    // Constants and modes survive the settings; broken constants don't.
    p.set_keypad_mode(KeypadMode::Scientific);
    p.mode = AngleMode::Radians;
    let mut settings = p.settings();
    settings.constants.push(("sin".into(), 1.0));
    settings.constants.push(("x".into(), f64::NAN));
    let mut q = QuickCalcPalette::default();
    q.apply_settings(&settings);
    assert_eq!(q.constants, p.constants);
    assert_eq!(q.keypad_mode, KeypadMode::Scientific);
    assert_eq!(q.mode, AngleMode::Radians);
}

#[test]
fn unit_converter() {
    let mut p = QuickCalcPalette {
        unit_value: "3600".into(),
        ..Default::default()
    };
    assert_eq!(p.converted_literal().as_deref(), Some("3.6m"));
    // Inserted, it reads back in the drawing unit.
    p.input = p.converted_literal().unwrap();
    assert_eq!(p.preview(&ctx(&p)), Preview::Ok(Value::Number(3600.0)));
    p.set_unit_cat(UnitCat::Area);
    p.unit_value = "2500000".into();
    assert_eq!(p.converted_literal().as_deref(), Some("2.5"));
    p.set_unit_cat(UnitCat::Angle);
    p.unit_value = "180".into();
    assert!((p.converted().unwrap() - std::f64::consts::PI).abs() < 1e-12);
    p.unit_value = "abc".into();
    assert_eq!(p.converted(), None);
}
