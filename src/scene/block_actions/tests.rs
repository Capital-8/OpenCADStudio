use super::*;
use crate::scene::{block_distance_parameter as distance, block_point_parameter as point, Scene};
use codec::{DwgReader, DwgWriter, DxfReader, DxfWriter};
use std::io::Cursor;

fn fixture() -> (CadDocument, Handle, Vec<Handle>) {
    let mut scene = Scene::new();
    scene.define_block_raw(
        "ACTIONS",
        Vector3::ZERO,
        vec![
            EntityType::Line(codec::entities::Line::from_points(
                Vector3::UNIT_X,
                Vector3::new(2., 0., 0.),
            )),
            EntityType::Circle(codec::entities::Circle::from_center_radius(
                Vector3::new(4., 3., 0.),
                2.,
            )),
        ],
    );
    let block = scene.document.block_records.get("ACTIONS").unwrap();
    let (definition, handles) = (block.handle, block.entity_handles.clone());
    (scene.document, definition, handles)
}

fn orientation(
    doc: &mut CadDocument,
    def: Handle,
    kind: super::super::block_orientation_parameter::Kind,
) -> Handle {
    super::super::block_orientation_parameter::create(
        doc,
        def,
        kind,
        Vector3::ZERO,
        Vector3::new(10., 0., 0.),
        if kind == super::super::block_orientation_parameter::Kind::Rotation {
            Vector3::new(10., 0., 0.)
        } else {
            Vector3::new(-2., 2., 0.)
        },
    )
    .unwrap()
}

#[test]
fn block_actions_rotate_flip_native_bindings_and_marker_edits_roundtrip() {
    use super::super::block_orientation_parameter::Kind as O;
    let (mut doc, def, targets) = fixture();
    let r = orientation(&mut doc, def, O::Rotation);
    let f = orientation(&mut doc, def, O::Flip);
    assert!(parameters(&doc, def, Kind::Move).is_empty());
    assert!(parameters(&doc, def, Kind::Scale).is_empty());
    assert_eq!(all_parameters(&doc, def).len(), 2);
    let rotate = create(
        &mut doc,
        def,
        Kind::Rotate,
        r,
        &targets,
        DVec3::new(3., 4., 0.),
    )
    .unwrap();
    let flip = create(
        &mut doc,
        def,
        Kind::Flip,
        f,
        &targets,
        DVec3::new(5., 6., 0.),
    )
    .unwrap();
    assert!(create(&mut doc, def, Kind::Rotate, f, &targets, DVec3::ZERO).is_err());
    for handle in [rotate, flip] {
        super::super::block_authoring::edit::move_part(
            &mut doc,
            def,
            handle,
            super::super::block_authoring::edit::Part::Point,
            Vector3::new(7., 8., 0.),
        )
        .unwrap();
    }
    for saved in [
        DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&doc).unwrap()))
            .read()
            .unwrap(),
        DxfReader::from_reader(Cursor::new(DxfWriter::new(&doc).write_to_vec().unwrap()))
            .unwrap()
            .read()
            .unwrap(),
    ] {
        assert_eq!(actions(&saved, def).len(), 2);
        for handle in [rotate, flip] {
            assert_eq!(saved.objects[&handle], doc.objects[&handle]);
        }
        for (_, action, _, _) in actions(&saved, def) {
            assert_eq!(action.display_location, Vector3::new(7., 8., 0.));
            assert_eq!(action.dependencies, targets);
        }
    }
}

#[test]
fn block_actions_rotate_flip_saved_values_compose_copy_and_roundtrip() {
    use super::super::block_orientation_parameter::Kind as O;
    let (mut doc, def, targets) = fixture();
    let r = orientation(&mut doc, def, O::Rotation);
    let f = orientation(&mut doc, def, O::Flip);
    create(&mut doc, def, Kind::Rotate, r, &[targets[0]], DVec3::ZERO).unwrap();
    create(&mut doc, def, Kind::Flip, f, &[targets[0]], DVec3::ZERO).unwrap();
    let a = doc
        .add_entity(EntityType::Insert(codec::entities::Insert::new(
            "ACTIONS",
            Vector3::ZERO,
        )))
        .unwrap();
    let b = doc
        .add_entity(EntityType::Insert(codec::entities::Insert::new(
            "ACTIONS",
            Vector3::new(30., 0., 0.),
        )))
        .unwrap();
    let baseline = doc.clone();
    doc = instance::evaluate(&doc, a, r, DVec3::new(0., 50., 0.))
        .unwrap()
        .unwrap();
    assert!(instance_line(&doc, a).distance(&Vector3::UNIT_Y) < 1e-8);
    doc = instance::evaluate_value(&doc, a, f, Value::Flip(true))
        .unwrap()
        .unwrap();
    assert!(instance_line(&doc, a).distance(&-Vector3::UNIT_Y) < 1e-8);
    assert!(instance::evaluate_value(&doc, a, f, Value::Flip(true))
        .unwrap()
        .is_none());
    let props = doc.dynamic_block_properties(def, Some(a)).unwrap();
    assert!(props
        .iter()
        .any(|p| p.parameter_handle == f && p.value == BlockEvalValue::Short(1)));
    assert!(props.iter().any(|p|p.parameter_handle==r&&matches!(p.value,BlockEvalValue::Real(v) if (v-std::f64::consts::FRAC_PI_2).abs()<1e-8)));
    let variants = [
        doc.clone(),
        DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&doc).unwrap()))
            .read()
            .unwrap(),
        DxfReader::from_reader(Cursor::new(DxfWriter::new(&doc).write_to_vec().unwrap()))
            .unwrap()
            .read()
            .unwrap(),
    ];
    for saved in variants {
        let mut next = instance::evaluate(&saved, a, r, DVec3::new(0., -50., 0.))
            .unwrap()
            .unwrap();
        assert!(instance_line(&next, a).distance(&Vector3::UNIT_Y) < 1e-8);
        for _ in 0..6 {
            next = instance::evaluate_value(&next, a, f, Value::Flip(false))
                .unwrap()
                .unwrap();
            assert!(instance_line(&next, a).distance(&-Vector3::UNIT_Y) < 1e-8);
            next = instance::evaluate_value(&next, a, f, Value::Flip(true))
                .unwrap()
                .unwrap();
            assert!(instance_line(&next, a).distance(&Vector3::UNIT_Y) < 1e-8);
        }
        assert_eq!(instance_line(&next, b), Vector3::UNIT_X);
        assert_eq!(start(&next, targets[0]), start(&baseline, targets[0]));
        let mut scene = Scene::new();
        scene.document = next;
        let c = scene.copy_entities(
            &[a],
            &crate::command::EntityTransform::Translate(DVec3::new(40., 0., 0.)),
        )[0];
        let next = instance::evaluate_value(&scene.document, c, f, Value::Flip(false))
            .unwrap()
            .unwrap();
        assert!(instance_line(&next, c).distance(&-Vector3::UNIT_Y) < 1e-8);
        assert!(instance_line(&next, a).distance(&Vector3::UNIT_Y) < 1e-8);
        let resaved = DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&next).unwrap()))
            .read()
            .unwrap();
        assert!(instance_line(&resaved, c).distance(&-Vector3::UNIT_Y) < 1e-8);
        let fp = parameters(&resaved, def, Kind::Flip).remove(0);
        assert_eq!(
            instance::value(&resaved, c, &fp).unwrap(),
            Value::Flip(false)
        );
        assert_eq!(
            instance::value(&resaved, a, &fp).unwrap(),
            Value::Flip(true)
        );
    }
}

#[test]
fn block_actions_rotate_flip_test_baseline_reversible_and_invalid_atomic() {
    use super::super::block_orientation_parameter::Kind as O;
    let (mut doc, def, targets) = fixture();
    let r = orientation(&mut doc, def, O::Rotation);
    let f = orientation(&mut doc, def, O::Flip);
    let action = create(&mut doc, def, Kind::Rotate, r, &targets, DVec3::ZERO).unwrap();
    create(&mut doc, def, Kind::Flip, f, &targets, DVec3::ZERO).unwrap();
    let baseline = doc.clone();
    assert!(evaluate(&mut doc, &baseline, def, r, DVec3::ZERO).is_err());
    assert_eq!(doc, baseline);
    assert!(evaluate_value(&mut doc, &baseline, def, r, Value::Flip(true)).is_err());
    assert!(evaluate_value(&mut doc, &baseline, def, f, Value::Point(DVec3::ONE)).is_err());
    assert_eq!(doc, baseline);
    evaluate(&mut doc, &baseline, def, r, DVec3::new(0., 10., 0.)).unwrap();
    evaluate_value(&mut doc, &baseline, def, f, Value::Flip(true)).unwrap();
    assert!(start(&doc, targets[0]).distance(&-Vector3::UNIT_Y) < 1e-8);
    evaluate_value(&mut doc, &baseline, def, f, Value::Flip(false)).unwrap();
    evaluate(&mut doc, &baseline, def, r, DVec3::new(10., 0., 0.)).unwrap();
    assert!(start(&doc, targets[0]).distance(&Vector3::UNIT_X) < 1e-8);
    let ObjectType::DynamicBlock(o) = doc.objects.get_mut(&action).unwrap() else {
        panic!()
    };
    let DynamicBlockData::RotateAction(a) = &mut o.data else {
        panic!()
    };
    a.action.dependent = false;
    let unsupported = doc.clone();
    assert!(evaluate(&mut doc, &unsupported, def, r, DVec3::new(0., 10., 0.)).is_err());
    assert_eq!(doc, unsupported);
}
fn linear(doc: &mut CadDocument, def: Handle) -> Handle {
    distance::create(
        doc,
        def,
        distance::Kind::Linear,
        Vector3::ZERO,
        Vector3::new(10., 0., 0.),
        Vector3::new(5., 2., 0.),
    )
    .unwrap()
}
fn start(doc: &CadDocument, h: Handle) -> Vector3 {
    let EntityType::Line(l) = doc.get_entity(h).unwrap() else {
        panic!()
    };
    l.start
}

fn instance_line(doc: &CadDocument, insert: Handle) -> Vector3 {
    let EntityType::Insert(i) = doc.get_entity(insert).unwrap() else {
        panic!()
    };
    start(
        doc,
        doc.block_records.get(&i.block_name).unwrap().entity_handles[0],
    )
}

#[test]
fn block_actions_saved_xy_polar_cache_metadata_and_rejections() {
    for kind in [distance::Kind::XY, distance::Kind::Polar] {
        let (mut doc, def, handles) = fixture();
        let p = distance::create(
            &mut doc,
            def,
            kind,
            Vector3::new(2., 3., 0.),
            Vector3::new(12., 13., 0.),
            Vector3::new(5., 6., 0.),
        )
        .unwrap();
        create(&mut doc, def, Kind::Move, p, &[handles[0]], DVec3::ZERO).unwrap();
        let a = doc
            .add_entity(EntityType::Insert(codec::entities::Insert::new(
                "ACTIONS",
                Vector3::ZERO,
            )))
            .unwrap();
        let original = doc.clone();
        let mut doc = instance::evaluate(&doc, a, p, DVec3::new(20., 18., 0.))
            .unwrap()
            .unwrap();
        assert_eq!(instance_line(&doc, a), Vector3::new(9., 5., 0.));
        let parameter = parameters(&doc, def, Kind::Move).remove(0);
        assert_eq!(
            instance::current(&doc, a, &parameter).unwrap(),
            DVec3::new(20., 18., 0.)
        );
        let r = doc
            .objects
            .values_mut()
            .find_map(|o| match o {
                ObjectType::XRecord(r) => Some(r),
                _ => None,
            })
            .unwrap();
        r.entries.push(codec::objects::XRecordEntry::string(
            1,
            "retain opaque metadata",
        ));
        doc = instance::evaluate(&doc, a, p, DVec3::new(22., 21., 0.))
            .unwrap()
            .unwrap();
        assert!(doc.objects.values().any(|o|matches!(o,ObjectType::XRecord(r) if r.entries.iter().any(|e|e.value.as_string()==Some("retain opaque metadata")))));
        let reopened = DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&doc).unwrap()))
            .read()
            .unwrap();
        assert_eq!(
            instance::current(&reopened, a, &parameter).unwrap(),
            DVec3::new(22., 21., 0.)
        );
        assert_eq!(start(&reopened,handles[0]),start(&original,handles[0]));
        assert!(instance::evaluate(&doc, a, p, DVec3::splat(f64::NAN)).is_err());
        let mut invalid = doc.clone();
        let EntityType::Insert(i) = invalid.get_entity_mut(a).unwrap() else {
            panic!()
        };
        i.set_x_scale(2.);
        assert!(instance::definition(&invalid, a).is_none());
        assert!(instance::evaluate(&invalid, a, p, DVec3::ONE).is_err());
    }
}

#[test]
fn block_actions_saved_references_independent_absolute_and_roundtrip() {
    let (mut doc, def, handles) = fixture();
    let p = linear(&mut doc, def);
    let q = point::create(&mut doc, def, Vector3::ZERO, Vector3::UNIT_Y).unwrap();
    create(&mut doc, def, Kind::Scale, p, &[handles[1]], DVec3::ZERO).unwrap();
    create(&mut doc, def, Kind::Move, q, &[handles[0]], DVec3::ZERO).unwrap();
    let a = doc
        .add_entity(EntityType::Insert(codec::entities::Insert::new(
            "ACTIONS",
            Vector3::ZERO,
        )))
        .unwrap();
    let b = doc
        .add_entity(EntityType::Insert(codec::entities::Insert::new(
            "ACTIONS",
            Vector3::new(30., 0., 0.),
        )))
        .unwrap();
    let original = doc.clone();
    doc = instance::evaluate(&doc, a, q, DVec3::new(3., 4., 0.))
        .unwrap()
        .unwrap();
    doc = instance::evaluate(&doc, a, p, DVec3::new(20., 7., 0.))
        .unwrap()
        .unwrap();
    assert_eq!(instance_line(&doc, a), Vector3::new(4., 4., 0.));
    assert_eq!(instance_line(&doc, b), Vector3::UNIT_X);
    for h in &handles {
        assert_eq!(doc.get_entity(*h), original.get_entity(*h));
    }
    for (h, o) in &original.objects {
        assert_eq!(doc.objects.get(h), Some(o));
    }
    let mut saved = vec![doc.clone()];
    saved.push(
        DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&doc).unwrap()))
            .read()
            .unwrap(),
    );
    saved.push(
        DxfReader::from_reader(Cursor::new(DxfWriter::new(&doc).write_to_vec().unwrap()))
            .unwrap()
            .read()
            .unwrap(),
    );
    for doc in saved {
        let parameter = parameters(&doc, def, Kind::Move)
            .into_iter()
            .find(|x| x.handle == p)
            .unwrap();
        assert_eq!(
            instance::current(&doc, a, &parameter).unwrap(),
            DVec3::new(20., 0., 0.)
        );
        let props = doc.dynamic_block_properties(def, Some(a)).unwrap();
        assert!(props
            .iter()
            .any(|x| x.parameter_handle == p && x.value == BlockEvalValue::Real(20.)));
        assert!(instance::evaluate(&doc, a, p, DVec3::new(20., 0., 0.))
            .unwrap()
            .is_none());
        let next = instance::evaluate(&doc, a, q, DVec3::new(-2., 1., 0.))
            .unwrap()
            .unwrap();
        assert_eq!(instance_line(&next, a), Vector3::new(-1., 1., 0.));
        let EntityType::Insert(i) = next.get_entity(a).unwrap() else {
            panic!()
        };
        let EntityType::Circle(c) = next
            .get_entity(
                next.block_records
                    .get(&i.block_name)
                    .unwrap()
                    .entity_handles[1],
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(c.radius, 4.);
        assert_eq!(instance_line(&next, b), Vector3::UNIT_X);
        let resaved = DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&next).unwrap()))
            .read()
            .unwrap();
        assert_eq!(instance_line(&resaved, a), Vector3::new(-1., 1., 0.));
        let point = parameters(&resaved, def, Kind::Move)
            .into_iter()
            .find(|x| x.handle == q)
            .unwrap();
        assert_eq!(
            instance::current(&resaved, a, &point).unwrap(),
            DVec3::new(-2., 1., 0.)
        );
        // A copied reference may share its evaluated block, but must isolate on edit.
        let mut scene = Scene::new();
        scene.document = next.clone();
        let c = scene.copy_entities(
            &[a],
            &crate::command::EntityTransform::Translate(DVec3::new(40., 0., 0.)),
        )[0];
        let copy = instance::evaluate(&scene.document, c, q, DVec3::new(7., 8., 0.))
            .unwrap()
            .unwrap();
        assert_eq!(instance_line(&copy, c), Vector3::new(8., 8., 0.));
        assert_eq!(instance_line(&copy, a), Vector3::new(-1., 1., 0.));
        assert_eq!(
            instance::current(&copy, c, &parameter).unwrap(),
            DVec3::new(20., 0., 0.)
        );
        let reset = instance::evaluate(&next, a, q, DVec3::ZERO)
            .unwrap()
            .unwrap();
        assert_eq!(instance_line(&reset, a), Vector3::UNIT_X);
    }
}
#[test]
fn block_actions_native_bindings_roundtrip_and_append_preserves_neighbors() {
    let (mut doc, def, handles) = fixture();
    let p = linear(&mut doc, def);
    for kind in [Kind::Move, Kind::Scale] {
        create(
            &mut doc,
            def,
            kind,
            p,
            &[handles[0]],
            DVec3::new(3., 4., 0.),
        )
        .unwrap();
    }
    let graph = authoring::graph(&doc, def).unwrap().unwrap();
    let ObjectType::DynamicBlock(o) = &doc.objects[&graph] else {
        panic!()
    };
    let DynamicBlockData::EvaluationGraph(g) = &o.data else {
        panic!()
    };
    assert!(g
        .nodes
        .iter()
        .all(|n| authoring::graph::valid_lists(g, n.id)));
    let dwg = DwgReader::from_stream(Cursor::new(DwgWriter::write_to_vec(&doc).unwrap()))
        .read()
        .unwrap();
    let dxf = DxfReader::from_reader(Cursor::new(DxfWriter::new(&doc).write_to_vec().unwrap()))
        .unwrap()
        .read()
        .unwrap();
    for saved in [dwg, dxf] {
        assert_eq!(actions(&saved, def).len(), 2);
        for (h, _, _, _) in actions(&doc, def) {
            assert_eq!(saved.objects[&h], doc.objects[&h]);
        }
        let mut test = saved.clone();
        assert!(evaluate(&mut test, &saved, def, p, DVec3::new(20., 5., 0.)).unwrap());
        assert_eq!(start(&test, handles[0]), Vector3::new(22., 0., 0.));
        assert_eq!(test.get_entity(handles[1]), saved.get_entity(handles[1]));
    }
}
#[test]
fn block_actions_point_move_is_absolute_reversible_and_leaves_unselected_objects() {
    let (mut doc, def, handles) = fixture();
    let p = point::create(
        &mut doc,
        def,
        Vector3::new(2., 3., 0.),
        Vector3::new(5., 6., 0.),
    )
    .unwrap();
    create(&mut doc, def, Kind::Move, p, &[handles[0]], DVec3::ZERO).unwrap();
    let baseline = doc.clone();
    for _ in 0..8 {
        assert!(evaluate(&mut doc, &baseline, def, p, DVec3::new(12., -7., 0.)).unwrap());
        assert_eq!(start(&doc, handles[0]), Vector3::new(11., -10., 0.));
        assert!(!evaluate(&mut doc, &baseline, def, p, DVec3::new(12., -7., 0.)).unwrap());
        assert!(evaluate(&mut doc, &baseline, def, p, DVec3::new(2., 3., 0.)).unwrap());
        assert_eq!(start(&doc, handles[0]), start(&baseline, handles[0]));
    }
    assert_eq!(doc.get_entity(handles[1]), baseline.get_entity(handles[1]));
}
#[test]
fn block_actions_scale_uses_distance_ratio_and_parameter_base() {
    let (mut doc, def, handles) = fixture();
    let p = distance::create(
        &mut doc,
        def,
        distance::Kind::Polar,
        Vector3::new(4., 3., 0.),
        Vector3::new(14., 3., 0.),
        Vector3::new(9., 5., 0.),
    )
    .unwrap();
    create(&mut doc, def, Kind::Scale, p, &[handles[1]], DVec3::ZERO).unwrap();
    let baseline = doc.clone();
    evaluate(&mut doc, &baseline, def, p, DVec3::new(4., 23., 0.)).unwrap();
    let EntityType::Circle(c) = doc.get_entity(handles[1]).unwrap() else {
        panic!()
    };
    assert_eq!(c.center, Vector3::new(4., 3., 0.));
    assert_eq!(c.radius, 4.);
    let before = doc.clone();
    assert!(evaluate(&mut doc, &baseline, def, p, DVec3::new(4., 3., 0.)).is_err());
    assert_eq!(doc, before);
}
#[test]
fn block_actions_reject_invalid_selection_binding_and_nonfinite_without_mutation() {
    let (mut doc, def, handles) = fixture();
    let p = linear(&mut doc, def);
    let before = doc.clone();
    for (parameter, targets, location) in [
        (Handle::NULL, vec![handles[0]], DVec3::ZERO),
        (p, vec![], DVec3::ZERO),
        (p, vec![Handle::new(0xff_ff_ff)], DVec3::ZERO),
        (p, vec![handles[0]], DVec3::splat(f64::NAN)),
    ] {
        assert!(create(&mut doc, def, Kind::Move, parameter, &targets, location).is_err());
        assert_eq!(doc, before);
    }
    let action = create(&mut doc, def, Kind::Move, p, &[handles[0]], DVec3::ZERO).unwrap();
    let baseline = doc.clone();
    assert!(evaluate(&mut doc, &baseline, def, p, DVec3::splat(f64::NAN)).is_err());
    assert_eq!(doc, baseline);
    let mut restricted = doc.clone();
    let ObjectType::DynamicBlock(object) = restricted.objects.get_mut(&p).unwrap() else {
        panic!()
    };
    let DynamicBlockData::LinearParameter(parameter) = &mut object.data else {
        panic!()
    };
    parameter.value_set.maximum = 15.;
    parameter.value_set.flags |= 2;
    let restricted_baseline = restricted.clone();
    assert!(evaluate(
        &mut restricted,
        &restricted_baseline,
        def,
        p,
        DVec3::new(20., 0., 0.)
    )
    .is_err());
    assert_eq!(
        restricted, restricted_baseline,
        "unsupported value constraints are not silently ignored"
    );
    let ObjectType::DynamicBlock(o) = doc.objects.get_mut(&action).unwrap() else {
        panic!()
    };
    let DynamicBlockData::MoveAction(m) = &mut o.data else {
        panic!()
    };
    m.offsets.distance_multiplier = 0.5;
    let baseline = doc.clone();
    assert!(evaluate(&mut doc, &baseline, def, p, DVec3::new(15., 0., 0.)).is_err());
    assert_eq!(doc, baseline);
}
#[test]
fn block_actions_reframe_moves_markers_and_retains_targets_and_connections() {
    let (mut doc, def, handles) = fixture();
    let p = linear(&mut doc, def);
    let h = create(
        &mut doc,
        def,
        Kind::Scale,
        p,
        &[handles[0]],
        DVec3::new(3., 4., 0.),
    )
    .unwrap();
    let before = doc.objects[&h].clone();
    let t = Transform::from_translation(Vector3::new(-2., -3., 0.));
    assert_eq!(reframe(&mut doc, def, &t), 1);
    let ObjectType::DynamicBlock(o) = &doc.objects[&h] else {
        panic!()
    };
    let DynamicBlockData::ScaleAction(s) = &o.data else {
        panic!()
    };
    assert_eq!(s.action.action.display_location, Vector3::new(1., 1., 0.));
    let ObjectType::DynamicBlock(o) = before else {
        panic!()
    };
    let DynamicBlockData::ScaleAction(b) = o.data else {
        panic!()
    };
    assert_eq!(s.action.action.dependencies, b.action.action.dependencies);
    assert_eq!(s.connections, b.connections);
}
