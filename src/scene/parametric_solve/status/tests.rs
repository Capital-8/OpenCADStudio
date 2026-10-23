use super::super::super::named_parameters::DrivingValue;
use super::*;
use crate::scene::parametric_constraints::{ParametricConstraintSet, ParametricScope};
use codec::entities::{Arc, Circle, Ellipse, Line};

fn fixture() -> (codec::CadDocument, Vec<Handle>, ParametricConstraintSet) {
    let mut doc = codec::CadDocument::new();
    let mut handles = vec![];
    for entity in [
        EntityType::Line(Line::from_points(Vector3::ZERO, Vector3::UNIT_X)),
        EntityType::Line(Line::from_points(
            Vector3::new(3., 0., 0.),
            Vector3::new(4., 0., 0.),
        )),
        EntityType::Circle(Circle::from_center_radius(Vector3::new(8., 0., 0.), 2.)),
    ] {
        handles.push(doc.add_entity(entity).unwrap());
    }
    (
        doc,
        handles,
        ParametricConstraintSet::new(ParametricScope::ModelSpace),
    )
}

#[test]
fn block_constraint_status_distinguishes_independent_entities_without_mutation() {
    let (doc, handles, mut set) = fixture();
    set.add(
        ConstraintKind::Fixed,
        vec![ParametricRef::whole(handles[0])],
        None,
    );
    set.add(
        ConstraintKind::Horizontal,
        vec![ParametricRef::whole(handles[1])],
        None,
    );
    let before = doc.clone();
    let before_constraints = set.constraints.clone();
    let report = analyze(&doc, &ParameterTable::new(), &set);
    assert_eq!(report[&handles[0]], Status::Full);
    assert_eq!(report[&handles[1]], Status::Partial);
    assert_eq!(
        report
            .get(&handles[2])
            .copied()
            .unwrap_or(Status::Unconstrained),
        Status::Unconstrained
    );
    assert_eq!(doc, before);
    assert_eq!(set.constraints, before_constraints);
    assert_eq!(set.dof, None);
    assert!(set.conflicts.is_empty());
}

#[test]
fn block_constraint_status_respects_coupled_point_freedom_and_disabled_constraints() {
    let (doc, handles, mut set) = fixture();
    set.add(
        ConstraintKind::Fixed,
        vec![ParametricRef::whole(handles[0])],
        None,
    );
    set.add(
        ConstraintKind::Coincident,
        vec![
            ParametricRef::point(handles[0], 1),
            ParametricRef::point(handles[1], 0),
        ],
        None,
    );
    let report = analyze(&doc, &ParameterTable::new(), &set);
    assert_eq!(report[&handles[0]], Status::Full);
    assert_eq!(
        report[&handles[1]],
        Status::Partial,
        "the other endpoint remains free"
    );
    set.add(
        ConstraintKind::Fixed,
        vec![ParametricRef::point(handles[1], 1)],
        None,
    );
    assert_eq!(
        analyze(&doc, &ParameterTable::new(), &set)[&handles[1]],
        Status::Full
    );
    for constraint in &mut set.constraints {
        constraint.enabled = false;
    }
    assert!(analyze(&doc, &ParameterTable::new(), &set)
        .values()
        .all(|status| *status == Status::Unconstrained));
}

#[test]
fn block_constraint_status_detects_redundancy_and_inconsistent_targets() {
    let (doc, handles, mut set) = fixture();
    set.add(
        ConstraintKind::Horizontal,
        vec![ParametricRef::whole(handles[0])],
        None,
    );
    set.add(
        ConstraintKind::Horizontal,
        vec![ParametricRef::whole(handles[0])],
        None,
    );
    assert_eq!(
        analyze(&doc, &ParameterTable::new(), &set)[&handles[0]],
        Status::Over
    );
    let mut set = ParametricConstraintSet::new(ParametricScope::ModelSpace);
    for radius in [2., 4.] {
        set.add(
            ConstraintKind::Radius,
            vec![ParametricRef::whole(handles[2])],
            Some(DrivingValue::Literal(radius)),
        );
    }
    let before = doc.clone();
    assert_eq!(
        analyze(&doc, &ParameterTable::new(), &set)[&handles[2]],
        Status::Over
    );
    assert_eq!(doc, before);
}

#[test]
fn block_constraint_status_accounts_for_arc_intrinsic_equations() {
    let (mut doc, _, mut set) = fixture();
    let arc = doc
        .add_entity(EntityType::Arc(Arc::from_center_radius_angles(
            Vector3::ZERO,
            2.,
            0.,
            1.,
        )))
        .unwrap();
    set.add(
        ConstraintKind::Radius,
        vec![ParametricRef::whole(arc)],
        Some(DrivingValue::Literal(2.)),
    );
    assert_eq!(
        analyze(&doc, &ParameterTable::new(), &set)[&arc],
        Status::Partial
    );
    set.constraints.clear();
    set.add(ConstraintKind::Fixed, vec![ParametricRef::whole(arc)], None);
    assert_eq!(
        analyze(&doc, &ParameterTable::new(), &set)[&arc],
        Status::Full
    );
}

#[test]
fn block_constraint_status_leaves_unsupported_references_unavailable() {
    let (doc, handles, mut set) = fixture();
    set.add(
        ConstraintKind::Radius,
        vec![ParametricRef::whole(handles[0])],
        Some(DrivingValue::Literal(2.)),
    );
    assert_eq!(
        analyze(&doc, &ParameterTable::new(), &set)[&handles[0]],
        Status::Unavailable
    );
}

#[test]
fn block_constraint_status_fixed_ellipse_holds_its_focus_and_minor_axis() {
    let (mut doc, _, mut set) = fixture();
    let ellipse = doc
        .add_entity(EntityType::Ellipse(Ellipse::from_center_axes(
            Vector3::ZERO,
            Vector3::new(4., 1., 0.),
            0.5,
        )))
        .unwrap();
    set.add(
        ConstraintKind::Fixed,
        vec![ParametricRef::whole(ellipse)],
        None,
    );
    assert_eq!(
        analyze(&doc, &ParameterTable::new(), &set)[&ellipse],
        Status::Full
    );
}

#[test]
fn block_constraint_status_does_not_classify_entities_from_a_foreign_scope() {
    let (doc, handles, mut set) = fixture();
    set.scope = ParametricScope::Block(Handle::from(0xFFFFu64));
    set.add(
        ConstraintKind::Fixed,
        vec![ParametricRef::whole(handles[0])],
        None,
    );
    assert!(analyze(&doc, &ParameterTable::new(), &set).is_empty());
}
