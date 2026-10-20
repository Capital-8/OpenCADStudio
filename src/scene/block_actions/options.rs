//! Native displacement overrides shared by Move and Stretch actions.
use super::*;

pub(crate) fn offsets(doc: &CadDocument, action: Handle) -> Option<&BlockActionOffsets> {
    let ObjectType::DynamicBlock(object) = doc.objects.get(&action)? else {
        return None;
    };
    match &object.data {
        DynamicBlockData::MoveAction(action) => Some(&action.offsets),
        DynamicBlockData::StretchAction(action) => Some(&action.offsets),
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
    if *before == updated {
        return Ok(false);
    }
    let Some(ObjectType::DynamicBlock(object)) = doc.objects.get_mut(&action) else {
        unreachable!()
    };
    match &mut object.data {
        DynamicBlockData::MoveAction(action) => action.offsets = updated,
        DynamicBlockData::StretchAction(action) => action.offsets = updated,
        _ => unreachable!(),
    }
    Ok(true)
}
