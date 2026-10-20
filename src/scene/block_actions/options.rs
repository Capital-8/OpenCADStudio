//! Native displacement overrides and Polar Stretch rotation selections.
use super::*;

pub(crate) fn offsets(doc: &CadDocument, action: Handle) -> Option<BlockActionOffsets> {
    let ObjectType::DynamicBlock(object) = doc.objects.get(&action)? else {
        return None;
    };
    match &object.data {
        DynamicBlockData::MoveAction(action) => Some(action.offsets.clone()),
        DynamicBlockData::StretchAction(action) => Some(action.offsets.clone()),
        DynamicBlockData::PolarStretchAction(action) => Some(BlockActionOffsets {
            distance_multiplier: action.distance_multiplier,
            angle_offset: action.angle_offset,
            flags: 0,
        }),
        _ => None,
    }
}

pub(crate) fn validate(offsets: &BlockActionOffsets) -> Result<(), &'static str> {
    if !offsets.distance_multiplier.is_finite() || !offsets.angle_offset.is_finite() {
        return Err("Action overrides must be finite numbers.");
    }
    if offsets.flags != 0 {
        return Err("This action uses an unsupported displacement mode.");
    }
    Ok(())
}

pub(crate) fn displacement(
    offsets: &BlockActionOffsets,
    delta: DVec3,
) -> Result<DVec3, &'static str> {
    validate(offsets)?;
    let (sin, cos) = offsets.angle_offset.sin_cos();
    let delta = DVec3::new(
        cos * delta.x - sin * delta.y,
        sin * delta.x + cos * delta.y,
        delta.z,
    ) * offsets.distance_multiplier;
    delta
        .is_finite()
        .then_some(delta)
        .ok_or("Action overrides produce non-finite geometry.")
}

pub(crate) fn set(
    doc: &mut CadDocument,
    definition: Handle,
    action: Handle,
    multiplier: f64,
    angle: f64,
) -> Result<bool, &'static str> {
    if !actions(doc, definition)
        .iter()
        .any(|(handle, ..)| *handle == action)
    {
        return Err("Select an action belonging to the active block definition.");
    }
    let before = offsets(doc, action).ok_or("This action has no displacement overrides.")?;
    let updated = BlockActionOffsets {
        distance_multiplier: multiplier,
        angle_offset: angle,
        flags: before.flags,
    };
    validate(&updated)?;
    if before == updated {
        return Ok(false);
    }
    let Some(ObjectType::DynamicBlock(object)) = doc.objects.get_mut(&action) else {
        unreachable!()
    };
    match &mut object.data {
        DynamicBlockData::MoveAction(action) => action.offsets = updated,
        DynamicBlockData::StretchAction(action) => action.offsets = updated,
        DynamicBlockData::PolarStretchAction(action) => {
            action.distance_multiplier = multiplier;
            action.angle_offset = angle;
        }
        _ => unreachable!(),
    }
    Ok(true)
}

pub(crate) fn set_rotate_only(
    doc: &mut CadDocument,
    definition: Handle,
    action: Handle,
    targets: &[Handle],
) -> Result<bool, &'static str> {
    if !actions(doc, definition)
        .iter()
        .any(|(handle, _, kind, _)| *handle == action && *kind == Kind::PolarStretch)
    {
        return Err("Select a Polar Stretch action belonging to the active block definition.");
    }
    let Some(ObjectType::DynamicBlock(object)) = doc.objects.get(&action) else {
        unreachable!()
    };
    let DynamicBlockData::PolarStretchAction(native) = &object.data else {
        unreachable!()
    };
    super::stretch::validate_polar(doc, native)?;
    let mut targets = targets.to_vec();
    targets.sort();
    targets.dedup();
    if targets
        .iter()
        .any(|handle| !native.action.dependencies.contains(handle))
    {
        return Err("Select rotate-only objects from this action's selection set.");
    }
    let mut before = native.handles.clone();
    before.sort();
    if targets == before {
        return Ok(false);
    }
    let rebound = before
        .iter()
        .copied()
        .filter(|handle| !targets.contains(handle))
        .collect::<Vec<_>>();
    let plane = native.action.display_location.z;
    let added = super::stretch::bind_existing(doc, &rebound, &native.points, plane)?;
    let mut staged = native.clone();
    staged.handles = targets;
    staged.bindings.retain(|binding| {
        !staged.handles.contains(&binding.handle) && !rebound.contains(&binding.handle)
    });
    staged.bindings.extend(added);
    staged.bindings.sort_by_key(|binding| binding.handle);
    super::stretch::validate_polar(doc, &staged)?;
    let Some(ObjectType::DynamicBlock(object)) = doc.objects.get_mut(&action) else {
        unreachable!()
    };
    object.data = DynamicBlockData::PolarStretchAction(staged);
    Ok(true)
}
