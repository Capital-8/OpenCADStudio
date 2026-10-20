//! A native Stretch action freezes its vertex membership at authoring time.
use super::*;
use codec::objects::{BlockPolarStretchAction, BlockStretchAction, BlockStretchHandle};
use codec::types::Vector2;

pub(crate) fn control_points(e: &EntityType) -> Result<Vec<DVec3>, &'static str> {
    let points = match e {
        EntityType::Line(l) => vec![v(l.start), v(l.end)],
        EntityType::Arc(a) => {
            if a.normal.x.abs() > 1e-10 || a.normal.y.abs() > 1e-10 {
                return Err("Stretch requires an arc parallel to the parameter's XY plane.");
            }
            vec![v(a.start_point_wcs()), v(a.end_point_wcs())]
        }
        EntityType::Circle(c) => vec![v(codec::types::Matrix3::arbitrary_axis(c.normal) * c.center)],
        EntityType::Ellipse(e) => vec![v(e.center)],
        EntityType::Point(p) => vec![v(p.location)],
        EntityType::Text(t) => vec![v(codec::types::Matrix3::arbitrary_axis(t.normal) * t.insertion_point)],
        EntityType::MText(t) => vec![v(t.insertion_point)],
        EntityType::LwPolyline(p) => {
            let p = crate::entities::curve::lwpolyline_world_xy(p)
                .ok_or("Stretch requires a planar XY lightweight polyline.")?;
            p.vertices
                .iter()
                .map(|v| DVec3::new(v.location.x, v.location.y, p.elevation))
                .collect()
        }
        _ => return Err("Select a supported Stretch object."),
    };
    if points.is_empty() || points.iter().any(|p| !p.is_finite()) {
        return Err("Stretch targets need finite vertices.");
    }
    Ok(points)
}

pub(crate) fn bind(
    doc: &CadDocument,
    targets: &[Handle],
    corners: [DVec3; 2],
    plane: f64,
) -> Result<(Vec<Vector2>, Vec<BlockStretchHandle>), &'static str> {
    if corners
        .iter()
        .any(|p| !p.is_finite() || (p.z - plane).abs() > 1e-8)
    {
        return Err("Specify finite stretch-frame corners in the parameter's XY plane.");
    }
    let low = corners[0].min(corners[1]);
    let high = corners[0].max(corners[1]);
    if high.x - low.x < 1e-9 || high.y - low.y < 1e-9 {
        return Err("The stretch frame needs a nonzero width and height.");
    }
    let points = vec![Vector2::new(low.x, low.y), Vector2::new(high.x, high.y)];
    let bindings = bind_existing(doc, targets, &points, plane)?;
    Ok((points, bindings))
}

pub(crate) fn bind_existing(
    doc: &CadDocument,
    targets: &[Handle],
    frame: &[Vector2],
    plane: f64,
) -> Result<Vec<BlockStretchHandle>, &'static str> {
    let polygon = outline(frame, plane);
    if polygon.len() < 3 || polygon.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err("The Stretch frame is invalid.");
    }
    let inside = |p: DVec3| {
        let mut crossing = false;
        for (a, b) in polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .take(polygon.len())
        {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let cross = (p.x - a.x) * dy - (p.y - a.y) * dx;
            if cross.abs() < 1e-8
                && p.x >= a.x.min(b.x) - 1e-8
                && p.x <= a.x.max(b.x) + 1e-8
                && p.y >= a.y.min(b.y) - 1e-8
                && p.y <= a.y.max(b.y) + 1e-8
            {
                return true;
            }
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                crossing = !crossing;
            }
        }
        crossing
    };
    let mut bindings = vec![];
    let mut unique = targets.to_vec();
    unique.sort();
    unique.dedup();
    for handle in unique {
        let points = control_points(
            doc.get_entity(handle)
                .ok_or("A Stretch target is unavailable.")?,
        )?;
        if points.iter().any(|p| (p.z - plane).abs() > 1e-8) {
            return Err("Stretch targets must lie in the parameter's XY plane.");
        }
        let indexes = points
            .into_iter()
            .enumerate()
            .filter(|(_, p)| inside(*p))
            .map(|(i, _)| i as i32)
            .collect::<Vec<_>>();
        if !indexes.is_empty() {
            bindings.push(BlockStretchHandle { handle, indexes });
        }
    }
    Ok(bindings)
}

pub(crate) fn validate_options(a: &BlockStretchAction) -> Result<(), &'static str> {
    options::validate(&a.offsets)?;
    if !a.codes.is_empty()
        || !a.action.action_ids.is_empty()
        || a.connections[0].code != a.connections[1].code
        || !matches!(
            [
                a.connections[0].name.as_str(),
                a.connections[1].name.as_str()
            ],
            ["XDelta", "YDelta"]
                | ["BaseXDelta", "BaseYDelta"]
                | ["EndXDelta", "BaseYDelta"]
                | ["BaseXDelta", "EndYDelta"]
                | ["EndXDelta", "EndYDelta"]
        )
    {
        return Err("This Stretch action uses options or parameter bindings not supported yet.");
    }
    Ok(())
}

pub(crate) fn validate(doc: &CadDocument, a: &BlockStretchAction) -> Result<(), &'static str> {
    validate_options(a)?;
    if a.points.len() < 2
        || a.points
            .iter()
            .any(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        return Err("The Stretch frame is invalid.");
    }
    let mut seen = std::collections::BTreeSet::new();
    for binding in &a.handles {
        if !a.action.dependencies.contains(&binding.handle) || !seen.insert(binding.handle) {
            return Err("The Stretch action has invalid target bindings.");
        }
        let count = control_points(
            doc.get_entity(binding.handle)
                .ok_or("A Stretch target is unavailable.")?,
        )?
        .len();
        let unique = binding
            .indexes
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != binding.indexes.len()
            || binding
                .indexes
                .iter()
                .any(|i| *i < 0 || *i as usize >= count)
        {
            return Err("A Stretch vertex binding no longer matches its object.");
        }
    }
    // Selected objects without a partial vertex binding move as a whole.
    for h in &a.action.dependencies {
        control_points(
            doc.get_entity(*h)
                .ok_or("A Stretch target is unavailable.")?,
        )?;
    }
    Ok(())
}

pub(crate) fn validate_polar(
    doc: &CadDocument,
    a: &BlockPolarStretchAction,
) -> Result<(), &'static str> {
    let connection_code = a.connections[0].code;
    let connections = [
        "BaseXDelta",
        "BaseYDelta",
        "Base",
        "End",
        "UpdatedBase",
        "UpdatedEnd",
    ]
    .map(|name| codec::objects::BlockConnection {
        code: connection_code,
        name: name.into(),
    });
    if a.distance_multiplier != 1.
        || a.angle_offset != 0.
        || a.connections != connections
        || !a.codes.is_empty()
        || !a.extra.is_empty()
        || !a.action.action_ids.is_empty()
        || a.points.len() < 2
        || a.points
            .iter()
            .any(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        return Err("This Polar Stretch action uses options not supported yet.");
    }
    let mut handles = a.handles.clone();
    handles.sort();
    handles.dedup();
    if handles != a.handles || handles.iter().any(|h| !a.action.dependencies.contains(h)) {
        return Err("The Polar Stretch action has invalid target bindings.");
    }
    let mut seen = std::collections::BTreeSet::new();
    for binding in &a.bindings {
        if !a.action.dependencies.contains(&binding.handle) || !seen.insert(binding.handle) {
            return Err("The Polar Stretch action has invalid vertex bindings.");
        }
        let count = control_points(
            doc.get_entity(binding.handle)
                .ok_or("A Polar Stretch target is unavailable.")?,
        )?
        .len();
        let unique = binding
            .indexes
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != binding.indexes.len()
            || binding
                .indexes
                .iter()
                .any(|i| *i < 0 || *i as usize >= count)
        {
            return Err("A Polar Stretch vertex binding no longer matches its object.");
        }
    }
    for h in &a.action.dependencies {
        control_points(
            doc.get_entity(*h)
                .ok_or("A Polar Stretch target is unavailable.")?,
        )?;
    }
    Ok(())
}

pub(crate) fn apply(
    e: &mut EntityType,
    a: &BlockStretchAction,
    delta: DVec3,
) -> Result<(), &'static str> {
    if delta == DVec3::ZERO { return Ok(()); }
    let Some(binding) = a
        .handles
        .iter()
        .find(|b| b.handle == e.common().handle && !b.indexes.is_empty())
    else {
        crate::scene::view::dispatch::apply_transform(
            e,
            &crate::command::EntityTransform::Translate(delta),
        );
        return Ok(());
    };
    match e {
        EntityType::Arc(arc) => {
            if !crate::modules::draw::modify::stretch::stretch_arc_endpoints(
                arc, binding.indexes.contains(&0), binding.indexes.contains(&1), delta,
            ) {
                return Err("The Stretch displacement collapses the arc chord.");
            }
        }
        EntityType::Circle(_) | EntityType::Ellipse(_) | EntityType::Point(_)
        | EntityType::Text(_) | EntityType::MText(_) => {
            crate::scene::view::dispatch::apply_transform(
                e, &crate::command::EntityTransform::Translate(delta),
            );
        }
        EntityType::Line(l) => {
            for index in &binding.indexes {
                let at = match index {
                    0 => &mut l.start,
                    1 => &mut l.end,
                    _ => return Err("Invalid Stretch line endpoint."),
                };
                *at = *at + native(delta);
            }
        }
        EntityType::LwPolyline(p) => {
            let mut world = crate::entities::curve::lwpolyline_world_xy(p)
                .ok_or("Stretch requires a planar XY lightweight polyline.")?;
            let before = world
                .vertices
                .iter()
                .map(|v| [v.location.x, v.location.y])
                .collect::<Vec<_>>();
            for index in &binding.indexes {
                let v = world
                    .vertices
                    .get_mut(*index as usize)
                    .ok_or("Invalid Stretch polyline vertex.")?;
                v.location.x += delta.x;
                v.location.y += delta.y;
            }
            let after = world
                .vertices
                .iter()
                .map(|v| [v.location.x, v.location.y])
                .collect::<Vec<_>>();
            for (i, v) in world.vertices.iter_mut().enumerate() {
                v.bulge = crate::modules::draw::modify::stretch::held_bulge(
                    v.bulge,
                    &before,
                    &after,
                    i,
                    world.is_closed,
                );
            }
            *p = world;
        }
        _ => return Err("Unsupported Stretch target."),
    }
    Ok(())
}

/// Apply the authored polar displacement to only the vertices frozen in the
/// Polar Stretch bindings; targets without a partial binding move as a whole.
pub(crate) fn apply_polar(
    e: &mut EntityType,
    bindings: &[BlockStretchHandle],
    base: DVec3,
    old_end: DVec3,
    new_end: DVec3,
) -> Result<(), &'static str> {
    let (old_vec, new_vec) = (old_end - base, new_end - base);
    if old_vec.length() <= 1e-9 || new_vec.length() <= 1e-9 {
        return Err("The Polar parameter needs a nonzero distance.");
    }
    let action = BlockStretchAction {
        handles: bindings.to_vec(),
        ..Default::default()
    };
    apply(e, &action, new_end - old_end)
}

pub(crate) fn outline(points: &[Vector2], z: f64) -> Vec<Vector3> {
    let points = if points.len() == 2 {
        let (a, b) = (points[0], points[1]);
        vec![a, Vector2::new(b.x, a.y), b, Vector2::new(a.x, b.y)]
    } else {
        points.to_vec()
    };
    points
        .into_iter()
        .map(|p| Vector3::new(p.x, p.y, z))
        .collect()
}
