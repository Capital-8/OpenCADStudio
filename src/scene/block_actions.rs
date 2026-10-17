//! Native Move/Scale/Stretch/Rotate/Flip authoring and deterministic block evaluation.
use super::block_authoring::{self as authoring, *};
use codec::objects::{
    BlockAction, BlockActionOffsets, BlockActionWithBasePoint, BlockBasePointAction,
    BlockConnection, BlockElement, BlockEvalValue, BlockMoveAction, BlockFlipAction,
    BlockStretchAction, DynamicBlockData, ObjectType,
};
use codec::types::{Transform, Vector3};
use codec::{CadDocument, EntityType, Handle};
use glam::DVec3;
pub(crate) mod instance;
mod stretch;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Move,
    Scale,
    Rotate,
    Flip,
    Stretch,
}
impl Kind {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "MOVE" | "M" => Some(Self::Move),
            "SCALE" | "S" => Some(Self::Scale),
            "ROTATE" | "R" => Some(Self::Rotate),
            "FLIP" | "F" => Some(Self::Flip),
            "STRETCH" | "ST" => Some(Self::Stretch),
            _ => None,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Scale => "Scale",
            Self::Rotate => "Rotate",
            Self::Flip => "Flip",
            Self::Stretch => "Stretch",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Parameter {
    pub handle: Handle,
    pub id: i32,
    pub grip: Handle,
    pub name: String,
    pub base: DVec3,
    pub end: DVec3,
    pub linear: bool,
    pub point: bool,
    pub xy: bool,
    pub rotation: bool,
    pub flip: bool,
    pub reference: DVec3,
}
pub(crate) fn v(p: Vector3) -> DVec3 {
    DVec3::new(p.x, p.y, p.z)
}
pub(crate) fn native(p: DVec3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Point(DVec3),
    Flip(bool),
}

pub(crate) fn all_parameters(doc: &CadDocument, definition: Handle) -> Vec<Parameter> {
    let mut result = parameters(doc, definition, Kind::Move);
    result.extend(parameters(doc, definition, Kind::Rotate));
    result.extend(parameters(doc, definition, Kind::Flip));
    result.sort_by_key(|p| p.id);
    result
}

pub(crate) fn compatible(p: &Parameter, kind: Kind) -> bool {
    match kind {
        Kind::Move => !p.rotation && !p.flip,
        Kind::Stretch => !p.rotation && !p.flip && !p.point,
        Kind::Scale => !p.rotation && !p.flip && !p.point && !p.xy,
        Kind::Rotate => p.rotation,
        Kind::Flip => p.flip,
    }
}

fn flip_component(doc: &CadDocument, p: &Parameter) -> Result<Handle, &'static str> {
    let Some(ObjectType::DynamicBlock(o)) = doc.objects.get(&p.grip) else {
        return Err("Missing Flip grip.");
    };
    let DynamicBlockData::FlipGrip(g) = &o.data else {
        return Err("Missing Flip grip.");
    };
    let candidates = doc
        .objects
        .iter()
        .filter_map(|(h, x)| match x {
            ObjectType::DynamicBlock(x) if x.owner == o.owner => match &x.data {
                DynamicBlockData::GripLocationComponent(c)
                    if c.eval.node_id == g.combined_state && c.expression == "UpdatedFlip" =>
                {
                    Some(*h)
                }
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err("The Flip state binding is unavailable.");
    }
    Ok(candidates[0])
}

pub(crate) fn value(doc: &CadDocument, p: &Parameter) -> Result<Value, &'static str> {
    if !p.flip {
        return Ok(Value::Point(current(doc, p)));
    }
    let h = flip_component(doc, p)?;
    let ObjectType::DynamicBlock(o) = &doc.objects[&h] else {
        unreachable!()
    };
    let DynamicBlockData::GripLocationComponent(c) = &o.data else {
        unreachable!()
    };
    match c.eval.value {
        BlockEvalValue::Short(0) => Ok(Value::Flip(false)),
        BlockEvalValue::Short(1) => Ok(Value::Flip(true)),
        _ => Err("The Flip state is not zero or one."),
    }
}

pub(crate) fn set_value(
    doc: &mut CadDocument,
    p: &Parameter,
    value: Value,
) -> Result<(), &'static str> {
    if let Value::Flip(state) = value {
        if !p.flip {
            return Err("Select a Flip parameter.");
        }
        let h = flip_component(doc, p)?;
        let Some(ObjectType::DynamicBlock(o)) = doc.objects.get_mut(&h) else {
            unreachable!()
        };
        let DynamicBlockData::GripLocationComponent(c) = &mut o.data else {
            unreachable!()
        };
        c.eval.value = BlockEvalValue::Short(i16::from(state));
        return Ok(());
    }
    let Value::Point(position) = value else {
        unreachable!()
    };
    let Some(ObjectType::DynamicBlock(o)) = doc.objects.get_mut(&p.grip) else {
        return Err("Missing parameter grip.");
    };
    match &mut o.data {
        DynamicBlockData::XYGrip(g)
        | DynamicBlockData::PolarGrip(g)
        | DynamicBlockData::RotationGrip(g) => g.location = native(position),
        DynamicBlockData::LinearGrip(g) => g.grip.location = native(position),
        _ => return Err("Unsupported parameter grip."),
    }
    Ok(())
}

pub(crate) fn grip_position(p: &Parameter, value: Value) -> DVec3 {
    match value {
        Value::Point(at) => at,
        Value::Flip(_) => p.reference,
    }
}

/// A Flip arrow points across its reflection line and reverses with its state.
pub(crate) fn grip_direction(p: &Parameter, value: Value) -> Option<DVec3> {
    let Value::Flip(flipped) = value else {
        return None;
    };
    let across = DVec3::Z.cross(p.end - p.base).try_normalize()?;
    Some(if flipped { -across } else { across })
}

pub(crate) fn parameters(doc: &CadDocument, definition: Handle, kind: Kind) -> Vec<Parameter> {
    let Ok(Some(graph)) = authoring::graph(doc, definition) else {
        return vec![];
    };
    let mut result = vec![];
    for (handle, p) in super::block_point_parameter::parameters(doc, definition) {
        if kind == Kind::Move {
            result.push(Parameter {
                handle,
                id: p.parameter.parameter.element.eval.node_id,
                grip: Handle::NULL,
                name: p.position_name.clone(),
                base: v(p.parameter.definition_point),
                end: v(p.parameter.definition_point),
                linear: false,
                point: true,
                xy: false,
                rotation: false,
                flip: false,
                reference: DVec3::ZERO,
            });
        }
    }
    for p in super::block_distance_parameter::parameters(doc, definition) {
        if matches!(kind, Kind::Rotate | Kind::Flip) {
            continue;
        }
        let xy = p.kind == super::block_distance_parameter::Kind::XY;
        if kind == Kind::Scale && xy {
            continue;
        }
        result.push(Parameter {
            handle: p.handle,
            id: p.native.parameter.element.eval.node_id,
            grip: Handle::NULL,
            name: p.labels.first().cloned().unwrap_or_default(),
            base: v(p.native.definition_base_point),
            end: v(p.native.definition_end_point),
            linear: p.kind == super::block_distance_parameter::Kind::Linear,
            point: false,
            xy,
            rotation: false,
            flip: false,
            reference: DVec3::ZERO,
        });
    }
    for p in super::block_orientation_parameter::parameters(doc, definition) {
        use super::block_orientation_parameter::Kind as O;
        if !matches!(
            (kind, p.kind),
            (Kind::Rotate, O::Rotation) | (Kind::Flip, O::Flip)
        ) {
            continue;
        }
        result.push(Parameter {
            handle: p.handle,
            id: p.native.parameter.element.eval.node_id,
            grip: Handle::NULL,
            name: p.label,
            base: v(p.native.definition_base_point),
            end: v(p.native.definition_end_point),
            linear: false,
            point: false,
            xy: false,
            rotation: kind == Kind::Rotate,
            flip: kind == Kind::Flip,
            reference: v(p.reference),
        });
    }
    // Resolve the actual linked grip, rather than assuming expression IDs are contiguous.
    result.retain_mut(|p| {
        let grip_id = if p.rotation || p.flip {
            let Some(ObjectType::DynamicBlock(o)) = doc.objects.get(&p.handle) else {
                return false;
            };
            match &o.data {
                DynamicBlockData::RotationParameter(p) => p.parameter.property_states[1],
                DynamicBlockData::FlipParameter(p) => p.flags_96,
                _ => return false,
            }
        } else if p.point {
            super::block_point_parameter::parameters(doc, definition)
                .into_iter()
                .find(|(h, _)| *h == p.handle)
                .unwrap()
                .1
                .parameter
                .property_count
        } else {
            super::block_distance_parameter::parameters(doc, definition)
                .into_iter()
                .find(|q| q.handle == p.handle)
                .unwrap()
                .native
                .properties[2]
                .connections
                .first()
                .map(|c| c.code)
                .unwrap_or(-1)
        };
        let grips = doc
            .objects
            .iter()
            .filter_map(|(h, o)| {
                let ObjectType::DynamicBlock(o) = o else {
                    return None;
                };
                if o.owner != graph {
                    return None;
                }
                let grip = match &o.data {
                    DynamicBlockData::XYGrip(g)
                    | DynamicBlockData::PolarGrip(g)
                    | DynamicBlockData::RotationGrip(g) => g,
                    DynamicBlockData::FlipGrip(g) => &g.grip,
                    DynamicBlockData::LinearGrip(g) => &g.grip,
                    _ => return None,
                };
                (grip.element.eval.node_id == grip_id).then_some(*h)
            })
            .collect::<Vec<_>>();
        if grips.len() != 1
            || !p.base.is_finite()
            || !p.end.is_finite()
            || (!p.point && (p.end.distance(p.base) < 1e-9 || !p.end.distance(p.base).is_finite()))
        {
            return false;
        }
        p.grip = grips[0];
        if p.flip {
            if let Some(ObjectType::DynamicBlock(o)) = doc.objects.get(&p.grip) {
                if let DynamicBlockData::FlipGrip(g) = &o.data {
                    let at = v(g.grip.location);
                    if at.distance(p.base) > 1e-8 {
                        p.reference = at;
                    }
                }
            }
            if p.reference.distance(p.base) < 1e-8 {
                p.reference = p.end;
            }
        }
        true
    });
    result.sort_by_key(|p| p.id);
    result
}

pub(crate) fn supported(e: &EntityType) -> bool {
    matches!(
        e,
        EntityType::Line(_)
            | EntityType::Circle(_)
            | EntityType::Arc(_)
            | EntityType::LwPolyline(_)
            | EntityType::Ellipse(_)
            | EntityType::Point(_)
            | EntityType::Text(_)
            | EntityType::MText(_)
    )
}
#[cfg(test)]
pub(crate) fn create(
    doc: &mut CadDocument,
    definition: Handle,
    kind: Kind,
    parameter: Handle,
    targets: &[Handle],
    location: DVec3,
) -> Result<Handle, &'static str> {
    create_with_frame(doc, definition, kind, parameter, targets, location, None)
}

pub(crate) fn create_with_frame(
    doc: &mut CadDocument,
    definition: Handle,
    kind: Kind,
    parameter: Handle,
    targets: &[Handle],
    location: DVec3,
    frame: Option<[DVec3; 2]>,
) -> Result<Handle, &'static str> {
    if !location.is_finite() {
        return Err("Specify a finite action location.");
    }
    let p = parameters(doc, definition, kind)
        .into_iter()
        .find(|p| p.handle == parameter)
        .ok_or("Select a compatible parameter belonging to this block.")?;
    if targets.is_empty() {
        return Err("Select at least one object for the action.");
    }
    let members = &doc
        .block_records
        .iter()
        .find(|b| b.handle == definition)
        .ok_or("The block definition is unavailable.")?
        .entity_handles;
    if targets.iter().any(|h| {
        !members.contains(h)
            || doc
                .get_entity(*h)
                .is_none_or(|e| !supported(e) || e.common().owner_handle != definition)
    }) {
        return Err("This milestone supports lines, polylines, arcs, circles, ellipses, points and text belonging to this block.");
    }
    let stretch_binding = if kind == Kind::Stretch {
        if (location.z - p.end.z).abs() > 1e-8 {
            return Err("Place the Stretch marker in the parameter's XY plane.");
        }
        Some(stretch::bind(
            doc,
            targets,
            frame.ok_or("Specify a stretch frame before selecting objects.")?,
            p.end.z,
        )?)
    } else {
        None
    };
    let mut staged = doc.clone();
    let (graph, slots) = authoring::prepare(&mut staged, definition)?;
    let (dxf, cpp) = match kind {
        Kind::Move => ("BLOCKMOVEACTION", "AcDbBlockMoveAction"),
        Kind::Scale => ("BLOCKSCALEACTION", "AcDbBlockScaleAction"),
        Kind::Rotate => ("BLOCKROTATEACTION", "AcDbBlockRotationAction"),
        Kind::Flip => ("BLOCKFLIPACTION", "AcDbBlockFlipAction"),
        Kind::Stretch => ("BLOCKSTRETCHACTION", "AcDbBlockStretchAction"),
    };
    register_classes(&mut staged, &[(dxf, cpp, 55)]);
    let handle = staged.allocate_handle();
    let mut dependencies = targets.to_vec();
    dependencies.sort();
    dependencies.dedup();
    let action = BlockAction {
        element: BlockElement {
            eval: expression(slots.eval, NO_VALUE, BlockEvalValue::None),
            name: format!("{}{}", kind.name(), slots.eval),
            major: MAJOR,
            minor: MINOR,
            eed_1071: 0,
        },
        display_location: native(location),
        dependencies,
        action_ids: vec![],
    };
    let connection = |name: &str| BlockConnection {
        code: p.id,
        name: name.into(),
    };
    let data = match kind {
        Kind::Stretch => {
            let (points, handles) = stretch_binding.unwrap();
            DynamicBlockData::StretchAction(BlockStretchAction {
                action,
                connections: [connection("EndXDelta"), connection("EndYDelta")],
                points,
                handles,
                codes: vec![],
                offsets: BlockActionOffsets {
                    distance_multiplier: 1.,
                    angle_offset: 0.,
                    flags: 0,
                },
            })
        }
        Kind::Move => DynamicBlockData::MoveAction(BlockMoveAction {
            action,
            connections: if p.point {
                [connection("XDelta"), connection("YDelta")]
            } else {
                [connection("EndXDelta"), connection("EndYDelta")]
            },
            offsets: BlockActionOffsets {
                distance_multiplier: 1.,
                angle_offset: 0.,
                flags: 0,
            },
        }),
        Kind::Flip => DynamicBlockData::FlipAction(BlockFlipAction {
            action,
            connections: [
                connection("Flip"),
                connection("UpdatedFlip"),
                connection("UpdatedBase"),
                connection("UpdatedEnd"),
            ],
        }),
        Kind::Rotate => DynamicBlockData::RotateAction(BlockBasePointAction {
            action: BlockActionWithBasePoint {
                action,
                offset: Vector3::ZERO,
                connections: vec![connection("UpdatedBaseX"), connection("UpdatedBaseY")],
                dependent: true,
                base_point: Vector3::ZERO,
            },
            connections: vec![connection("AngleDelta")],
            dwg_tail_byte: 0,
        }),
        Kind::Scale => DynamicBlockData::ScaleAction(BlockBasePointAction {
            action: BlockActionWithBasePoint {
                action,
                offset: Vector3::ZERO,
                connections: vec![connection("UpdatedBaseX"), connection("UpdatedBaseY")],
                dependent: true,
                base_point: Vector3::ZERO,
            },
            connections: vec![
                connection("Scale"),
                connection("XScale"),
                connection("YScale"),
            ],
            dwg_tail_byte: 0,
        }),
    };
    staged.objects.insert(
        handle,
        ObjectType::DynamicBlock(dynamic(handle, graph, dxf, cpp, data)),
    );
    authoring::append_single(&mut staged, graph, slots, handle);
    authoring::graph::bind_ids(&mut staged, graph, slots.eval, &[p.id])?;
    *doc = staged;
    Ok(handle)
}

/// Retarget an existing native action, retaining identity, graph and options.
pub(crate) fn retarget(
    doc: &mut CadDocument,
    definition: Handle,
    handle: Handle,
    targets: &[Handle],
    frame: Option<[DVec3; 2]>,
) -> Result<bool, &'static str> {
    let (_, action, kind, id) = actions(doc, definition)
        .into_iter()
        .find(|(h, _, _, _)| *h == handle)
        .ok_or("Select an action belonging to this block.")?;
    let mut targets = targets.to_vec();
    targets.sort();
    targets.dedup();
    if targets.is_empty() {
        return Err("An action needs at least one object.");
    }
    let members = &doc
        .block_records
        .iter()
        .find(|b| b.handle == definition)
        .ok_or("The definition is unavailable.")?
        .entity_handles;
    if targets.iter().any(|h| {
        !members.contains(h)
            || doc
                .get_entity(*h)
                .is_none_or(|e| e.common().owner_handle != definition || !supported(e))
    }) {
        return Err("Select supported drawing objects belonging to this block.");
    }
    let old_targets = action.dependencies.clone();
    let mut staged = doc.clone();
    let ObjectType::DynamicBlock(o) = staged.objects.get_mut(&handle).unwrap() else {
        unreachable!()
    };
    match &mut o.data {
        DynamicBlockData::StretchAction(a) => {
            let plane = parameters(doc, definition, Kind::Stretch)
                .into_iter()
                .find(|p| p.id == id)
                .ok_or("The Stretch parameter is unavailable.")?
                .end
                .z;
            stretch::validate_options(a)?;
            if let Some(corners) = frame {
                let (points, handles) = stretch::bind(doc, &targets, corners, plane)?;
                a.points = points;
                a.handles = handles;
            } else {
                a.handles.retain(|h| targets.contains(&h.handle));
                let mut retained = a.clone();
                retained.action.dependencies.retain(|h| targets.contains(h));
                stretch::validate(doc, &retained)?;
                let added = targets
                    .iter()
                    .copied()
                    .filter(|h| !old_targets.contains(h))
                    .collect::<Vec<_>>();
                a.handles
                    .extend(stretch::bind_existing(doc, &added, &a.points, plane)?);
                a.handles.sort_by_key(|h| h.handle);
            }
            a.action.dependencies = targets;
        }
        DynamicBlockData::MoveAction(a) => a.action.dependencies = targets,
        DynamicBlockData::FlipAction(a) => a.action.dependencies = targets,
        DynamicBlockData::ScaleAction(a) | DynamicBlockData::RotateAction(a) => {
            a.action.action.dependencies = targets
        }
        _ => return Err("This action's selection cannot be edited yet."),
    }
    if frame.is_some() && kind != Kind::Stretch {
        return Err("Only Stretch actions have a stretch frame.");
    }
    if staged == *doc {
        return Ok(false);
    }
    *doc = staged;
    Ok(true)
}

pub(crate) fn actions(
    doc: &CadDocument,
    definition: Handle,
) -> Vec<(Handle, &BlockAction, Kind, i32)> {
    let Ok(Some(graph)) = authoring::graph(doc, definition) else {
        return vec![];
    };
    let ObjectType::DynamicBlock(o) = &doc.objects[&graph] else {
        return vec![];
    };
    let DynamicBlockData::EvaluationGraph(g) = &o.data else {
        return vec![];
    };
    g.nodes
        .iter()
        .filter_map(|n| {
            let ObjectType::DynamicBlock(o) = doc.objects.get(&n.expression)? else {
                return None;
            };
            if o.owner != graph {
                return None;
            }
            match &o.data {
                DynamicBlockData::MoveAction(a) => {
                    Some((o.handle, &a.action, Kind::Move, a.connections[0].code))
                }
                DynamicBlockData::StretchAction(a) => {
                    Some((o.handle, &a.action, Kind::Stretch, a.connections[0].code))
                }
                DynamicBlockData::FlipAction(a) => {
                    Some((o.handle, &a.action, Kind::Flip, a.connections[0].code))
                }
                DynamicBlockData::RotateAction(a) => a
                    .connections
                    .first()
                    .map(|c| (o.handle, &a.action.action, Kind::Rotate, c.code)),
                DynamicBlockData::ScaleAction(a) => a
                    .connections
                    .first()
                    .map(|c| (o.handle, &a.action.action, Kind::Scale, c.code)),
                _ => None,
            }
        })
        .collect()
}
pub(crate) fn frame_outline(doc: &CadDocument, handle: Handle) -> Vec<Vector3> {
    let Some(ObjectType::DynamicBlock(o)) = doc.objects.get(&handle) else {
        return vec![];
    };
    let DynamicBlockData::StretchAction(a) = &o.data else {
        return vec![];
    };
    stretch::outline(&a.points, a.action.display_location.z)
}

pub(crate) fn reframe(doc: &mut CadDocument, definition: Handle, t: &Transform) -> usize {
    let handles = actions(doc, definition)
        .iter()
        .map(|(h, _, _, _)| *h)
        .collect::<Vec<_>>();
    for h in &handles {
        let ObjectType::DynamicBlock(o) = doc.objects.get_mut(h).unwrap() else {
            unreachable!()
        };
        match &mut o.data {
            DynamicBlockData::MoveAction(a) => {
                a.action.display_location = t.apply(a.action.display_location)
            }
            DynamicBlockData::StretchAction(a) => {
                let plane = a.action.display_location.z;
                a.action.display_location = t.apply(a.action.display_location);
                a.points = stretch::outline(&a.points, plane)
                    .into_iter()
                    .map(|p| {
                        let p = t.apply(p);
                        codec::types::Vector2::new(p.x, p.y)
                    })
                    .collect();
            }
            DynamicBlockData::FlipAction(a) => {
                a.action.display_location = t.apply(a.action.display_location)
            }
            DynamicBlockData::ScaleAction(a) | DynamicBlockData::RotateAction(a) => {
                a.action.action.display_location = t.apply(a.action.action.display_location);
                a.action.base_point = t.apply(a.action.base_point);
                a.action.offset = t.apply_rotation(a.action.offset);
            }
            _ => {}
        }
    }
    handles.len()
}

pub(crate) fn current(doc: &CadDocument, p: &Parameter) -> DVec3 {
    let Some(ObjectType::DynamicBlock(o)) = doc.objects.get(&p.grip) else {
        return p.end;
    };
    match &o.data {
        DynamicBlockData::XYGrip(g)
        | DynamicBlockData::PolarGrip(g)
        | DynamicBlockData::RotationGrip(g) => v(g.location),
        DynamicBlockData::LinearGrip(g) => v(g.grip.location),
        _ => p.end,
    }
}

/// Visibility can give the test INSERT an anonymous representation. Keep its
/// parallel members in sync, retaining their own handles and visibility flags.
pub(crate) fn sync_representation(
    doc: &mut CadDocument,
    definition: Handle,
    name: &str,
) -> Result<(), &'static str> {
    let original = doc
        .block_records
        .iter()
        .find(|b| b.handle == definition)
        .ok_or("The test definition is unavailable.")?;
    if original.name == name {
        return Ok(());
    }
    let source = original.entity_handles.clone();
    let target = doc
        .block_records
        .get(name)
        .ok_or("The test representation is unavailable.")?
        .entity_handles
        .clone();
    if source.len() != target.len() {
        return Err("The test representation no longer matches its definition.");
    }
    let affected = actions(doc, definition)
        .iter()
        .flat_map(|(_, a, _, _)| a.dependencies.iter().copied())
        .collect::<std::collections::BTreeSet<_>>();
    for (from, to) in source.into_iter().zip(target) {
        if !affected.contains(&from) {
            continue;
        }
        let mut entity = doc
            .get_entity(from)
            .cloned()
            .ok_or("An action target is unavailable.")?;
        let current = doc
            .get_entity_mut(to)
            .ok_or("An action representation target is unavailable.")?;
        *entity.common_mut() = current.common().clone();
        *current = entity;
    }
    Ok(())
}

/// Re-evaluate from the scratch document's immutable starting geometry. This
/// avoids incremental drift and makes repeated changes/order independent.
#[cfg(test)]
pub(crate) fn evaluate(
    doc: &mut CadDocument,
    baseline: &CadDocument,
    definition: Handle,
    parameter: Handle,
    position: DVec3,
) -> Result<bool, &'static str> {
    evaluate_value(doc, baseline, definition, parameter, Value::Point(position))
}

pub(crate) fn evaluate_value(
    doc: &mut CadDocument,
    baseline: &CadDocument,
    definition: Handle,
    parameter: Handle,
    input: Value,
) -> Result<bool, &'static str> {
    evaluate_impl(doc, baseline, definition, parameter, input, false)
}

pub(crate) fn rebuild(
    doc: &mut CadDocument,
    baseline: &CadDocument,
    definition: Handle,
) -> Result<(), &'static str> {
    let Some(p) = all_parameters(baseline, definition).into_iter().next() else {
        return Ok(());
    };
    let input = value(doc, &p)?;
    evaluate_impl(doc, baseline, definition, p.handle, input, true)?;
    Ok(())
}

fn evaluate_impl(
    doc: &mut CadDocument,
    baseline: &CadDocument,
    definition: Handle,
    parameter: Handle,
    input: Value,
    force: bool,
) -> Result<bool, &'static str> {
    let graph = authoring::graph(baseline, definition)?
        .ok_or("The block authoring graph is unavailable.")?;
    if baseline.objects.values().any(|o| {
        matches!(o,ObjectType::DynamicBlock(o) if o.owner==graph && matches!(&o.data,
        DynamicBlockData::PolarStretchAction(_)|DynamicBlockData::ArrayAction(_)))
    }) {
        return Err("This block contains action types not supported yet.");
    }
    let p = all_parameters(baseline, definition)
        .into_iter()
        .find(|p| p.handle == parameter)
        .ok_or("The block parameter is unavailable.")?;
    let input = match input {
        Value::Flip(_) if !p.flip => return Err("Select a Flip parameter."),
        Value::Point(_) if p.flip => return Err("Use the Flip grip to toggle its state."),
        Value::Point(mut position) => {
            if !position.is_finite() {
                return Err("Specify a finite parameter position.");
            }
            position.z = p.end.z;
            if p.linear {
                let axis = (p.end - p.base).normalize();
                let length = (position - p.base).dot(axis);
                if length < 1e-9 {
                    return Err("Specify a positive parameter length.");
                }
                position = p.base + axis * length;
            }
            if p.rotation {
                let delta = position - p.base;
                if delta.length() < 1e-9 {
                    return Err("Specify a rotation direction away from the parameter base.");
                }
                position = p.base + delta.normalize() * p.end.distance(p.base);
            }
            Value::Point(position)
        }
        value => value,
    };
    let previous = value(doc, &p)?;
    if !force
        && match (previous, input) {
            (Value::Flip(a), Value::Flip(b)) => a == b,
            (Value::Point(a), Value::Point(b)) => a.distance(b) < 1e-10,
            _ => false,
        }
    {
        return Ok(false);
    }
    let mut staged = doc.clone();
    set_value(&mut staged, &p, input)?;
    let all = actions(baseline, definition);
    let mut affected = std::collections::BTreeSet::new();
    for (h, a, kind, _) in &all {
        if *kind == Kind::Stretch {
            let ObjectType::DynamicBlock(o) = &baseline.objects[h] else {
                unreachable!()
            };
            let DynamicBlockData::StretchAction(s) = &o.data else {
                unreachable!()
            };
            stretch::validate(baseline, s)?;
        }
        affected.extend(a.dependencies.iter().copied());
    }
    for h in affected {
        let mut entity = baseline
            .get_entity(h)
            .cloned()
            .ok_or("An action target is unavailable.")?;
        if !supported(&entity) {
            return Err("This action contains an unsupported object type.");
        }
        // State flags may have changed through the existing visibility grip.
        if let Some(e) = staged.get_entity(h) {
            entity.common_mut().invisible = e.common().invisible;
        }
        for (action_handle, a, kind, id) in &all {
            if !a.dependencies.contains(&h) {
                continue;
            }
            let q = parameters(baseline, definition, *kind)
                .into_iter()
                .find(|p| p.id == *id)
                .ok_or("An action has an unsupported parameter binding.")?;
            if !supported_parameter(baseline, q.handle) {
                return Err("This parameter uses value constraints or chaining not supported yet.");
            }
            let end = current(&staged, &q);
            let transform = match kind {
                Kind::Move | Kind::Stretch => {
                    crate::command::EntityTransform::Translate(end - q.end)
                }
                Kind::Rotate => {
                    let a = end - q.base;
                    let b = q.end - q.base;
                    crate::command::EntityTransform::Rotate {
                        center: q.base,
                        axis: DVec3::Z,
                        angle_rad: a.y.atan2(a.x) - b.y.atan2(b.x),
                    }
                }
                Kind::Flip => {
                    if value(&staged, &q)? == Value::Flip(false) {
                        crate::command::EntityTransform::Translate(DVec3::ZERO)
                    } else {
                        crate::command::EntityTransform::Mirror {
                            p1: q.base,
                            p2: q.end,
                            working_normal: DVec3::Z,
                        }
                    }
                }
                Kind::Scale => {
                    let factor = end.distance(q.base) / q.end.distance(q.base);
                    if !factor.is_finite() || factor < 1e-9 {
                        return Err("Specify a positive scale distance.");
                    }
                    crate::command::EntityTransform::Scale {
                        center: q.base,
                        factor,
                    }
                }
            };
            // Imported actions with custom multipliers/offsets/chaining must
            // not silently behave like the simple actions authored here.
            if !a.action_ids.is_empty() {
                return Err("Chained actions are not supported yet.");
            }
            let ObjectType::DynamicBlock(o) = &baseline.objects[action_handle] else {
                unreachable!()
            };
            match &o.data {
                DynamicBlockData::MoveAction(m)
                    if m.offsets.distance_multiplier != 1.
                        || m.offsets.angle_offset != 0.
                        || m.offsets.flags != 0
                        || m.connections
                            != if q.point {
                                [
                                    BlockConnection {
                                        code: q.id,
                                        name: "XDelta".into(),
                                    },
                                    BlockConnection {
                                        code: q.id,
                                        name: "YDelta".into(),
                                    },
                                ]
                            } else {
                                [
                                    BlockConnection {
                                        code: q.id,
                                        name: "EndXDelta".into(),
                                    },
                                    BlockConnection {
                                        code: q.id,
                                        name: "EndYDelta".into(),
                                    },
                                ]
                            } =>
                {
                    return Err("This Move action uses options not supported yet.")
                }
                DynamicBlockData::FlipAction(f)
                    if f.connections
                        != ["Flip", "UpdatedFlip", "UpdatedBase", "UpdatedEnd"].map(|name| {
                            BlockConnection {
                                code: q.id,
                                name: name.into(),
                            }
                        }) =>
                {
                    return Err("This Flip action uses options not supported yet.")
                }
                DynamicBlockData::RotateAction(r)
                    if !r.action.dependent
                        || r.action.offset != Vector3::ZERO
                        || r.connections
                            != vec![BlockConnection {
                                code: q.id,
                                name: "AngleDelta".into(),
                            }]
                        || r.action.connections
                            != ["UpdatedBaseX", "UpdatedBaseY"].map(|name| BlockConnection {
                                code: q.id,
                                name: name.into(),
                            }) =>
                {
                    return Err("This Rotate action uses options not supported yet.")
                }
                DynamicBlockData::ScaleAction(s)
                    if !s.action.dependent
                        || s.action.offset != Vector3::ZERO
                        || s.connections
                            != ["Scale", "XScale", "YScale"].map(|name| BlockConnection {
                                code: q.id,
                                name: name.into(),
                            })
                        || s.action.connections
                            != ["UpdatedBaseX", "UpdatedBaseY"].map(|name| BlockConnection {
                                code: q.id,
                                name: name.into(),
                            }) =>
                {
                    return Err("This Scale action uses options not supported yet.")
                }
                _ => {}
            }
            if let DynamicBlockData::StretchAction(s) = &o.data {
                stretch::apply(&mut entity, s, end - q.end)?;
            } else {
                super::view::dispatch::apply_transform(&mut entity, &transform);
            }
            let bounds = entity.as_entity().bounding_box();
            if !v(bounds.min).is_finite() || !v(bounds.max).is_finite() {
                return Err("The parameter value produces non-finite geometry.");
            }
        }
        *staged
            .get_entity_mut(h)
            .ok_or("An action target was removed.")? = entity;
    }
    *doc = staged;
    Ok(true)
}

fn supported_parameter(doc: &CadDocument, handle: Handle) -> bool {
    let Some(ObjectType::DynamicBlock(o)) = doc.objects.get(&handle) else {
        return false;
    };
    let distance = |s: &codec::objects::BlockParameterValueSet| {
        s.flags == 1
            && s.minimum == 0.
            && s.maximum == 0.
            && s.increment == 0.
            && s.values.is_empty()
    };
    match &o.data {
        DynamicBlockData::FlipParameter(p) => !p.parameter.parameter.chain_actions,
        DynamicBlockData::RotationParameter(p) => {
            !p.parameter.parameter.chain_actions
                && p.value_set.flags == 3
                && p.value_set.minimum == 0.
                && p.value_set.maximum == std::f64::consts::TAU
                && p.value_set.increment == 0.
                && p.value_set.values.is_empty()
        }
        DynamicBlockData::PointParameter(p) => !p.parameter.parameter.chain_actions,
        DynamicBlockData::LinearParameter(p) => {
            !p.parameter.parameter.chain_actions && distance(&p.value_set)
        }
        DynamicBlockData::XYParameter(p) => {
            !p.parameter.parameter.chain_actions
                && distance(&p.x_value_set)
                && distance(&p.y_value_set)
        }
        DynamicBlockData::PolarParameter(p) => {
            !p.parameter.parameter.chain_actions
                && distance(&p.distance_value_set)
                && p.angle_value_set.flags == 3
                && p.angle_value_set.minimum == 0.
                && p.angle_value_set.maximum == std::f64::consts::TAU
                && p.angle_value_set.increment == 0.
                && p.angle_value_set.values.is_empty()
        }
        _ => false,
    }
}
