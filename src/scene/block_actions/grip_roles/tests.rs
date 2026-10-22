use super::*;
use crate::scene::{
    block_distance_parameter as distance,
    block_grips::{config, runtime},
    block_point_parameter as point,
};

#[test]
fn block_displacement_grip_roles_follow_native_key_point_bindings() {
    for parameter_kind in 0..4 {
        for kind in [Kind::Move, Kind::Stretch, Kind::PolarStretch] {
            if kind == Kind::PolarStretch && parameter_kind != 2 {
                continue;
            }
            let (mut doc, definition, targets) = super::super::tests::fixture();
            let handle = if parameter_kind == 0 {
                point::create(&mut doc, definition, Vector3::ZERO, Vector3::UNIT_Y).unwrap()
            } else {
                distance::create(
                    &mut doc,
                    definition,
                    match parameter_kind {
                        1 => distance::Kind::Linear,
                        2 => distance::Kind::Polar,
                        _ => distance::Kind::XY,
                    },
                    Vector3::ZERO,
                    Vector3::new(10., 10., 0.),
                    Vector3::UNIT_Y,
                )
                .unwrap()
            };
            let count = if parameter_kind == 0 {
                1
            } else if parameter_kind == 3 {
                4
            } else {
                2
            };
            config::set(&mut doc, definition, handle, Some(count)).unwrap();
            let p = all_parameters(&doc, definition)
                .into_iter()
                .find(|p| p.handle == handle)
                .unwrap();
            assert_eq!(mask(&doc, definition, &p), 0);
            let keys = if parameter_kind == 0 {
                vec![(None, 1)]
            } else if parameter_kind == 3 {
                vec![
                    (Some(XyKeyPoint::Base), 1),
                    (Some(XyKeyPoint::End), 2),
                    (Some(XyKeyPoint::X), 4),
                    (Some(XyKeyPoint::Y), 8),
                ]
            } else {
                vec![(Some(XyKeyPoint::Base), 1), (Some(XyKeyPoint::End), 2)]
            };
            let mut expected = 0;
            for (key, bit) in keys {
                let frame = [DVec3::new(0.5, -0.5, 0.), DVec3::new(1.5, 0.5, 0.)];
                let action = create_with_options(
                    &mut doc,
                    definition,
                    kind,
                    handle,
                    key,
                    &targets[..1],
                    DVec3::ZERO,
                    needs_frame(kind).then_some(frame),
                    None,
                )
                .unwrap();
                expected |= bit;
                assert_eq!(mask(&doc, definition, &p), expected);
                let roles = runtime::action_grips(&doc, definition, &p, Value::Point(p.end))
                    .into_iter()
                    .map(|g| g.role)
                    .collect::<Vec<_>>();
                assert_eq!(
                    roles,
                    (0..4)
                        .filter(|role| expected & (1 << role) != 0)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    config::locations(&doc, definition, handle).unwrap().len(),
                    count
                );
                for extension in ["dwg", "dxf"] {
                    let bytes = crate::io::save_to_bytes(&doc, extension, doc.version).unwrap();
                    let loaded =
                        crate::io::load_bytes(&format!("key-point.{extension}"), bytes).unwrap();
                    assert_eq!(mask(&loaded, definition, &p), expected);
                }
                let mut malformed = doc.clone();
                let ObjectType::DynamicBlock(object) = malformed.objects.get_mut(&action).unwrap()
                else {
                    panic!()
                };
                let connections = match &mut object.data {
                    DynamicBlockData::MoveAction(a) => &mut a.connections[..],
                    DynamicBlockData::StretchAction(a) => &mut a.connections[..],
                    DynamicBlockData::PolarStretchAction(a) => &mut a.connections[..],
                    _ => unreachable!(),
                };
                connections[1].code = -1;
                assert_eq!(mask(&malformed, definition, &p), expected & !bit);
                let unchanged = config::locations(&malformed, definition, handle).unwrap();
                assert_eq!(unchanged.len(), count);
            }
        }
    }
}

#[test]
#[ignore = "requires OCS_DYNAMIC_BLOCK_FIXTURES native fixture directory"]
fn block_key_point_fixture_suppresses_unassociated_base_grip() {
    let root = std::env::var("OCS_DYNAMIC_BLOCK_FIXTURES").expect("fixture root");
    let path = std::path::Path::new(&root).join("DXF-2018/closet.dxf");
    let bytes = std::fs::read(&path).unwrap();
    let doc = crate::io::load_bytes(path.to_str().unwrap(), bytes).unwrap();
    let handle = Handle::from(0x3A7ADu64);
    let (definition, p) = doc
        .block_records
        .iter()
        .find_map(|block| {
            all_parameters(&doc, block.handle)
                .into_iter()
                .find(|p| p.handle == handle)
                .map(|p| (block.handle, p))
        })
        .unwrap();
    assert_eq!(
        config::locations(&doc, definition, handle).unwrap().len(),
        2
    );
    assert_eq!(mask(&doc, definition, &p), 2);
    assert_eq!(
        runtime::action_grips(&doc, definition, &p, Value::Point(p.end))
            .into_iter()
            .map(|g| g.role)
            .collect::<Vec<_>>(),
        vec![1]
    );
    for extension in ["dwg", "dxf"] {
        let data = crate::io::save_to_bytes(&doc, extension, doc.version).unwrap();
        let loaded = crate::io::load_bytes(&format!("closet-roles.{extension}"), data).unwrap();
        assert_eq!(mask(&loaded, definition, &p), 2);
        assert_eq!(
            config::locations(&loaded, definition, handle)
                .unwrap()
                .len(),
            2
        );
    }
}
