use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Unconstrained,
    Partial,
    Full,
    Over,
    Unavailable,
}

impl Status {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Unconstrained => "Unconstrained",
            Self::Partial => "Partially constrained",
            Self::Full => "Fully constrained",
            Self::Over => "Over-constrained",
            Self::Unavailable => "Unavailable",
        }
    }
}

impl Scene {
    pub(crate) fn block_constraint_entity_status(
        &self,
        definition: Handle,
        entity: Handle,
    ) -> Status {
        if self
            .document
            .get_entity(entity)
            .is_none_or(|e| e.common().owner_handle != definition)
        {
            return Status::Unavailable;
        }
        let scope = super::super::parametric_constraints::ParametricScope::Block(definition);
        self.parametric_constraints
            .iter()
            .find(|set| set.scope == scope)
            .map(|set| {
                analyze(&self.document, &self.named_parameters, set)
                    .get(&entity)
                    .copied()
                    .unwrap_or(Status::Unavailable)
            })
            .unwrap_or(Status::Unconstrained)
    }
}

fn geometry_parameters(geometry: &EntityGeom) -> Vec<ParamId> {
    let line = |line: &GLine| vec![line.p1.x, line.p1.y, line.p2.x, line.p2.y];
    let arc = |arc: &GArc| {
        vec![
            arc.circle.center.x,
            arc.circle.center.y,
            arc.circle.rad,
            arc.start.x,
            arc.start.y,
            arc.end.x,
            arc.end.y,
            arc.start_angle,
            arc.end_angle,
        ]
    };
    match geometry {
        EntityGeom::Point(point) => vec![point.x, point.y],
        EntityGeom::Line(value)
        | EntityGeom::TextLine(value)
        | EntityGeom::Ray(value)
        | EntityGeom::XLine(value) => line(value),
        EntityGeom::Circle(circle) => vec![circle.center.x, circle.center.y, circle.rad],
        EntityGeom::Arc(value) => arc(value),
        EntityGeom::Ellipse(ellipse) => vec![
            ellipse.center.x,
            ellipse.center.y,
            ellipse.focus1.x,
            ellipse.focus1.y,
            ellipse.radmin,
        ],
        EntityGeom::Polyline { points, arcs, .. } => points
            .iter()
            .flat_map(|p| [p.x, p.y])
            .chain(arcs.iter().flatten().flat_map(|segment| arc(&segment.arc)))
            .collect(),
        EntityGeom::Spline { curve, .. } => curve.poles.iter().flat_map(|p| [p.x, p.y]).collect(),
    }
}

/// Analyze persistent equations in an isolated store, without edit-time pins,
/// write-back, or mutation of the document and constraint set.
pub(crate) fn analyze(
    document: &codec::CadDocument,
    drawing_parameters: &ParameterTable,
    set: &super::super::parametric_constraints::ParametricConstraintSet,
) -> HashMap<Handle, Status> {
    let parameters = if set.local_parameters.is_empty() {
        drawing_parameters
    } else {
        &set.local_parameters
    };
    let scope = set.scope.owner_handle(document);
    let mut result: HashMap<_, _> = document
        .entities()
        .filter(|entity| {
            entity.common().owner_handle == scope
                && !matches!(
                    entity,
                    EntityType::Block(_) | EntityType::BlockEnd(_) | EntityType::Viewport(_)
                )
        })
        .map(|entity| (entity.common().handle, Status::Unconstrained))
        .collect();
    let mut sys = System::new();
    let mut cache = HashMap::new();
    let mut owners = Vec::new();
    let mut unavailable = HashSet::new();
    for constraint in set.constraints.iter().filter(|c| c.enabled) {
        for reference in &constraint.refs {
            if let Some(status) = result.get_mut(&reference.entity) {
                *status = Status::Partial;
            }
        }
        if constraint.refs.iter().any(|reference| {
            document.get_entity(reference.entity).is_none_or(|entity| {
                entity.common().owner_handle != set.scope.owner_handle(document)
            })
        }) {
            unavailable.extend(constraint.refs.iter().map(|r| r.entity));
            continue;
        }
        let mut equations = build_constraint(
            document,
            &mut sys,
            &mut cache,
            parameters,
            constraint,
            None,
            None,
            &HashMap::new(),
        );
        // Fixed holds the complete ellipse. The solve path also retains its
        // focus-to-center offset; include that freedom here without importing
        // the temporary shape-retention equations used during editing.
        if constraint.kind == ConstraintKind::Fixed {
            if let Some(reference) = constraint.refs.first().filter(|r| r.marker.is_none()) {
                if let Some(EntityGeom::Ellipse(ellipse)) = cache.get(&reference.entity) {
                    for parameter in [ellipse.focus1.x, ellipse.focus1.y] {
                        let value = sys.store().get(parameter);
                        let target = sys.add_param(value, true);
                        equations.push(Rc::new(Equal::new(parameter, target, 1.0)));
                    }
                }
            }
        }
        if equations.is_empty() || !refs_share_supported_plane(document, &constraint.refs) {
            unavailable.extend(constraint.refs.iter().map(|r| r.entity));
            continue;
        }
        for equation in equations {
            owners.push((Rc::clone(&equation), constraint));
            sys.add_constraint(equation);
        }
    }
    add_intrinsic_arc_rules(&mut sys, &cache);
    // A text baseline's second point is an orientation guide. Its distance is
    // fixed by text layout, not an additional editable geometric freedom.
    for geometry in cache.values() {
        if let EntityGeom::TextLine(line) = geometry {
            let store = sys.store();
            let length = (store.get(line.p2.x) - store.get(line.p1.x))
                .hypot(store.get(line.p2.y) - store.get(line.p1.y));
            let target = sys.add_param(length, true);
            sys.add_constraint(Rc::new(P2PDistance::new(line.p1, line.p2, target)));
        }
    }
    let partitions = sys.partition();
    for partition in &partitions {
        if !partition
            .calc_residual(sys.store())
            .iter()
            .all(|value| value.is_finite())
            || !partition
                .calc_jacobi(sys.store())
                .iter()
                .all(|value| value.is_finite())
        {
            for (handle, geometry) in &cache {
                if geometry_parameters(geometry)
                    .iter()
                    .any(|p| partition.plist().contains(p))
                {
                    unavailable.insert(*handle);
                }
            }
            continue;
        }
        if solve_dl(partition, sys.store_mut()) == SolveStatus::Failed {
            let _ = solve_lm(partition, sys.store_mut());
        }
    }
    let mut pinned = HashSet::new();
    let mut over = HashSet::new();
    for partition in &partitions {
        let matrix = partition.calc_jacobi(sys.store());
        if !matrix.iter().all(|value| value.is_finite()) {
            for (handle, geometry) in &cache {
                if geometry_parameters(geometry)
                    .iter()
                    .any(|p| partition.plist().contains(p))
                {
                    unavailable.insert(*handle);
                }
            }
            continue;
        }
        if matrix.nrows() != 0 && matrix.ncols() != 0 {
            let tolerance = (matrix.norm() * 1e-10).max(f64::EPSILON);
            let decomposition = matrix.svd(false, true);
            if let Some(vt) = decomposition.v_t {
                // The row-space projector fixes a coordinate only if that
                // coordinate has no component in the Jacobian's null space.
                // One SVD per partition also handles coupled entities without
                // performing a fresh rank decomposition for each object.
                for (column, parameter) in partition.plist().iter().enumerate() {
                    let projection: f64 = decomposition
                        .singular_values
                        .iter()
                        .enumerate()
                        .filter(|(_, value)| **value > tolerance)
                        .map(|(row, _)| vt[(row, column)].powi(2))
                        .sum();
                    if projection >= 1.0 - 1e-8 {
                        pinned.insert(*parameter);
                    }
                }
            }
        }
        let diagnosis = kernel_constraints::diagnosis::diagnose(partition, sys.store());
        for row in diagnosis.redundant {
            if let Some((_, owner)) = owners
                .iter()
                .find(|(equation, _)| Rc::ptr_eq(equation, &partition.constraints()[row]))
            {
                over.extend(owner.refs.iter().map(|r| r.entity));
            }
        }
    }
    for (equation, owner) in &owners {
        let error = equation.error(sys.store());
        if !error.is_finite() {
            unavailable.extend(owner.refs.iter().map(|r| r.entity));
        } else if error.abs() > 1e-6 {
            over.extend(owner.refs.iter().map(|r| r.entity));
        }
    }
    for (handle, geometry) in &cache {
        let params = geometry_parameters(geometry);
        if !params.is_empty()
            && params
                .iter()
                .all(|p| sys.store().is_driven(*p) || pinned.contains(p))
        {
            result.insert(*handle, Status::Full);
        }
    }
    for handle in unavailable {
        if let Some(status) = result.get_mut(&handle) {
            *status = Status::Unavailable;
        }
    }
    for handle in over {
        if let Some(status) = result.get_mut(&handle) {
            *status = Status::Over;
        }
    }
    result
}

#[cfg(test)]
mod tests;

pub(super) fn add_intrinsic_arc_rules(sys: &mut System, cache: &HashMap<Handle, EntityGeom>) {
    let add_arc_rules = |sys: &mut System, arc: GArc, sweep: Option<f64>| {
        let curve = Rc::new(arc);
        sys.add_constraint(Rc::new(CurveValue::new(
            arc.start,
            arc.start.x,
            curve.clone(),
            arc.start_angle,
        )));
        sys.add_constraint(Rc::new(CurveValue::new(
            arc.start,
            arc.start.y,
            curve.clone(),
            arc.start_angle,
        )));
        sys.add_constraint(Rc::new(CurveValue::new(
            arc.end,
            arc.end.x,
            curve.clone(),
            arc.end_angle,
        )));
        sys.add_constraint(Rc::new(CurveValue::new(
            arc.end,
            arc.end.y,
            curve,
            arc.end_angle,
        )));
        if let Some(sweep) = sweep {
            let target = sys.add_param(sweep, true);
            sys.add_constraint(Rc::new(Difference::new(
                arc.start_angle,
                arc.end_angle,
                target,
            )));
        }
    };
    for geom in cache.values() {
        match geom {
            EntityGeom::Arc(arc) => add_arc_rules(sys, *arc, None),
            EntityGeom::Polyline { arcs, .. } => {
                for segment in arcs.iter().flatten() {
                    add_arc_rules(sys, segment.arc, Some(segment.sweep));
                }
            }
            _ => {}
        }
    }
}
