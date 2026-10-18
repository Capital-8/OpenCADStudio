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

fn press(p: &mut QuickCalcPalette, key: Key) -> Option<String> {
    let c = ctx(p);
    p.press(key, &c)
}

#[test]
fn submit_records_history_and_ans() {
    let mut p = QuickCalcPalette::default();
    let s = submit(&mut p, "2*wall + 3*90").unwrap();
    assert_eq!(s.value, Value::Number(870.0));
    assert_eq!(s.assign, None);
    assert_eq!(p.ans, Some(Value::Number(870.0)));
    assert!(p.input.is_empty());
    // A leading operator continues from ans, and the history says so.
    submit(&mut p, "/ 2").unwrap();
    assert_eq!(p.history[1].expr, "ans / 2");
    assert_eq!(p.ans, Some(Value::Number(435.0)));
    // An error keeps the input and changes nothing.
    assert!(submit(&mut p, "2 +").is_none());
    assert_eq!(p.input, "2 +");
    assert_eq!(p.history.len(), 2);
}

#[test]
fn assignment_is_reported_and_stored() {
    let mut p = QuickCalcPalette::default();
    let s = submit(&mut p, "height = 2800").unwrap();
    assert_eq!(s.assign.as_deref(), Some("height"));
    // A number goes to the drawing …
    assert_eq!(p.store("height", s.value), Some(2800.0));
    // … unless a constant has that name …
    p.constants.push(("brick".into(), 210.0));
    assert_eq!(p.store("brick", Value::Number(215.0)), None);
    assert_eq!(p.constants[0], ("brick".into(), 215.0));
    // … and a point lives for the session.
    let pt = Value::Point(DVec3::new(1.0, 2.0, 0.0));
    assert_eq!(p.store("p1", pt), None);
    assert_eq!(ctx(&p).vars["p1"], pt);
}

#[test]
fn context_layers_constants_drawing_and_session() {
    let mut p = QuickCalcPalette::default();
    p.constants.push(("wall".into(), 1.0));
    p.constants.push(("joint".into(), 10.0));
    let c = ctx(&p);
    assert_eq!(c.vars["wall"], Value::Number(300.0));
    assert_eq!(c.vars["joint"], Value::Number(10.0));
    p.mode = AngleMode::Radians;
    assert_eq!(ctx(&p).angle_mode, AngleMode::Radians);
}

#[test]
fn history_arrows() {
    let mut p = QuickCalcPalette::default();
    submit(&mut p, "1+1");
    submit(&mut p, "2+2");
    p.history_step(true);
    assert_eq!(p.input, "2+2");
    p.history_step(true);
    assert_eq!(p.input, "1+1");
    p.history_step(true);
    assert_eq!(p.input, "1+1");
    p.history_step(false);
    assert_eq!(p.input, "2+2");
    p.history_step(false);
    assert_eq!(p.input, "");
    assert_eq!(p.hist_idx, None);
}

#[test]
fn wrap_keys_take_the_whole_input_or_ans() {
    let mut p = QuickCalcPalette {
        input: "wall + 90".into(),
        ..Default::default()
    };
    press(&mut p, Key::Sqrt);
    assert_eq!(p.input, "sqrt(wall + 90)");
    p.input = "wall".into();
    press(&mut p, Key::Square);
    assert_eq!(p.input, "wall^2");
    p.input = "2+1".into();
    press(&mut p, Key::Inverse);
    assert_eq!(p.input, "1/(2+1)");
    // Empty with no ans: the start of the function is typed.
    p.input.clear();
    press(&mut p, Key::Sqrt);
    assert_eq!(p.input, "sqrt(");
    // Empty with an ans: it is used.
    submit(&mut p, "9");
    press(&mut p, Key::Cube);
    assert_eq!(p.input, "ans^3");
    p.input.clear();
    press(&mut p, Key::Sqrt);
    assert_eq!(p.input, "sqrt(ans)");
    press(&mut p, Key::Back);
    assert_eq!(p.input, "sqrt(ans");
    press(&mut p, Key::Clear);
    assert!(p.input.is_empty());
}

#[test]
fn memory_keys() {
    let mut p = QuickCalcPalette::default();
    assert!(press(&mut p, Key::MemStore).is_some());
    assert_eq!(p.memory, None);
    submit(&mut p, "870");
    press(&mut p, Key::MemStore);
    assert_eq!(p.memory, Some(870.0));
    p.input = "30".into();
    press(&mut p, Key::MemAdd);
    assert_eq!(p.memory, Some(900.0));
    p.input = "[1,2]".into();
    assert!(press(&mut p, Key::MemStore).is_some());
    assert_eq!(p.memory, Some(900.0));
    p.input.clear();
    press(&mut p, Key::MemRecall);
    assert_eq!(p.input, "900");
    press(&mut p, Key::MemClear);
    assert_eq!(p.memory, None);
}

#[test]
fn unit_converter() {
    let mut p = QuickCalcPalette {
        unit_value: "3600".into(),
        ..Default::default()
    };
    assert_eq!(p.converted(), Some(3.6));
    assert_eq!(p.converted_literal().as_deref(), Some("3.6m"));
    (p.unit_from, p.unit_to) = (p.unit_to, p.unit_from);
    assert_eq!(p.converted(), Some(3_600_000.0));
    p.set_unit_cat(UnitCat::Area);
    p.unit_value = "2500000".into();
    assert_eq!(p.converted(), Some(2.5));
    assert_eq!(p.converted_literal().as_deref(), Some("2.5"));
    p.set_unit_cat(UnitCat::Angle);
    p.unit_value = "180".into();
    assert!((p.converted().unwrap() - std::f64::consts::PI).abs() < 1e-12);
    p.unit_value = "abc".into();
    assert_eq!(p.converted(), None);
    // The literal reads back through the calculator in the drawing unit.
    p.set_unit_cat(UnitCat::Length);
    p.unit_value = "3600".into();
    p.input = p.converted_literal().unwrap();
    assert_eq!(p.preview(&ctx(&p)), Preview::Ok(Value::Number(3600.0)));
}

#[test]
fn input_classification() {
    assert!(continues_ans(" * 2"));
    assert!(!continues_ans("-2"));
    assert_eq!(assignment_target("wall = 300"), Some("wall"));
    assert_eq!(assignment_target("a == b"), None);
    assert_eq!(assignment_target("2 = 3"), None);
    assert!(has_length_suffix("1.2m + 350mm"));
    assert!(has_length_suffix("2'9\""));
    assert!(!has_length_suffix("3 * mid([0,0],[1,1])"));
    assert!(!has_length_suffix("90deg"));
}

#[test]
fn full_text_keeps_precision_and_reads_as_a_coordinate() {
    assert_eq!(full_text(&Value::Number(0.1 + 0.2)), "0.30000000000000004");
    assert_eq!(full_text(&Value::Point(DVec3::new(1.5, 2.0, 0.0))), "1.5,2");
    assert_eq!(full_text(&Value::Point(DVec3::new(1.0, 2.0, 3.0))), "1,2,3");
}

#[test]
fn settings_round_trip() {
    let mut p = QuickCalcPalette::default();
    p.constants.push(("brick".into(), 210.0));
    p.mode = AngleMode::Radians;
    p.toggle(Section::Units);
    let s = p.settings();
    let json = serde_json::to_string(&s).unwrap();
    let back: QuickCalcSettings = serde_json::from_str(&json).unwrap();
    let mut q = QuickCalcPalette::default();
    q.apply_settings(&back);
    assert_eq!(q.constants, p.constants);
    assert_eq!(q.mode, AngleMode::Radians);
    assert!(q.is_open(Section::Units));
    // A reserved or broken constant from a hand-edited file is dropped.
    let mut bad = back.clone();
    bad.constants.push(("sin".into(), 1.0));
    bad.constants.push(("x".into(), f64::NAN));
    q.apply_settings(&bad);
    assert_eq!(q.constants.len(), 1);
}
