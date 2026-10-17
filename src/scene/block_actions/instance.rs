//! Saved INSERT evaluation: definition geometry stays immutable; parameter
//! values live in the native per-reference AppDataCache.
use super::*;
use codec::objects::{Dictionary, XRecord, XRecordEntry, XRecordValue};

fn entry(doc: &CadDocument, owner: Handle, key: &str) -> Result<Option<Handle>, &'static str> {
    let Some(ObjectType::Dictionary(d)) = doc.objects.get(&owner) else {
        return Err("The reference contains an invalid parameter dictionary.");
    };
    Ok(d.entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, h)| *h))
}

fn cache(doc: &CadDocument, insert: Handle, id: i32) -> Result<Option<Handle>, &'static str> {
    let Some(mut owner) = doc.extension_dictionary_handle(insert) else {
        return Ok(None);
    };
    for key in [
        "AcDbBlockRepresentation",
        "AppDataCache",
        "ACAD_ENHANCEDBLOCKDATA",
    ] {
        let Some(next) = entry(doc, owner, key)? else {
            return Ok(None);
        };
        owner = next;
    }
    entry(doc, owner, &id.to_string())
}

pub(crate) fn current(
    doc: &CadDocument,
    insert: Handle,
    p: &Parameter,
) -> Result<DVec3, &'static str> {
    let Some(h) = cache(doc, insert, p.id)? else {
        return Ok(p.end);
    };
    let Some(ObjectType::XRecord(r)) = doc.objects.get(&h) else {
        return Err("The reference contains an invalid parameter record.");
    };
    if !r.entries_complete {
        return Err("The reference parameter cache is incomplete.");
    }
    let points = r
        .entries
        .iter()
        .filter_map(|e| e.value.as_point3d())
        .map(|(x, y, z)| DVec3::new(x, y, z))
        .collect::<Vec<_>>();
    if p.rotation
        && points
            .get(3)
            .is_none_or(|r| !r.is_finite() || r.distance(p.reference) > 1e-8)
    {
        return Err("The rotation reference point is unavailable or has been relocated.");
    }
    if p.xy {
        let (Some(a), Some(b)) = (points.first(), points.get(1)) else {
            return Err("The XY parameter cache has no finite endpoint.");
        };
        if !a.is_finite()
            || !b.is_finite()
            || (a.x - p.base.x).abs() > 1e-8
            || (b.y - p.base.y).abs() > 1e-8
            || (a.z - p.base.z).abs() > 1e-8
            || (b.z - p.end.z).abs() > 1e-8
        {
            return Err("This reference uses an unsupported XY parameter frame.");
        }
        return Ok(DVec3::new(b.x, a.y, b.z));
    }
    let index = usize::from(!p.point);
    if !p.point && points.first().is_some_and(|a| a.distance(p.base) > 1e-8) {
        return Err("This reference uses a relocated parameter base that is not supported yet.");
    }
    points
        .get(index)
        .copied()
        .filter(|v| v.is_finite())
        .ok_or("The reference parameter cache has no finite endpoint.")
}

pub(crate) fn value(
    doc: &CadDocument,
    insert: Handle,
    p: &Parameter,
) -> Result<Value, &'static str> {
    if !p.flip {
        return current(doc, insert, p).map(Value::Point);
    }
    let Some(h) = cache(doc, insert, p.id)? else {
        return Ok(Value::Flip(false));
    };
    let Some(ObjectType::XRecord(r)) = doc.objects.get(&h) else {
        return Err("Invalid Flip parameter record.");
    };
    if !r.entries_complete {
        return Err("The Flip parameter cache is incomplete.");
    }
    let points = r
        .entries
        .iter()
        .filter_map(|e| e.value.as_point3d())
        .map(|(x, y, z)| DVec3::new(x, y, z))
        .collect::<Vec<_>>();
    if points.len() < 2
        || !points[0].is_finite()
        || !points[1].is_finite()
        || points[0].distance(p.base) > 1e-8
        || points[1].distance(p.end) > 1e-8
    {
        return Err("This reference uses an unsupported Flip reflection line.");
    }
    let state = r.entries.iter().skip(4).rev().find_map(|e| match e.value {
        XRecordValue::Int16(s) => Some(s),
        _ => None,
    });
    match state {
        Some(0) => Ok(Value::Flip(false)),
        Some(1) => Ok(Value::Flip(true)),
        _ => Err("The Flip state is not zero or one."),
    }
}

fn ensure_dictionary(
    doc: &mut CadDocument,
    owner: Handle,
    key: &str,
) -> Result<Handle, &'static str> {
    if let Some(h) = entry(doc, owner, key)? {
        if matches!(doc.objects.get(&h), Some(ObjectType::Dictionary(_))) {
            return Ok(h);
        }
        return Err("The reference contains an invalid parameter dictionary.");
    }
    let h = doc.allocate_handle();
    let mut d = Dictionary::new();
    d.handle = h;
    d.owner = owner;
    d.reactors.push(owner);
    doc.objects.insert(h, ObjectType::Dictionary(d));
    if let Some(ObjectType::Dictionary(d)) = doc.objects.get_mut(&owner) {
        d.add_entry(key, h);
    }
    Ok(h)
}

fn save(
    doc: &mut CadDocument,
    insert: Handle,
    p: &Parameter,
    value: Value,
) -> Result<(), &'static str> {
    let mut owner = doc.ensure_extension_dictionary(insert);
    for key in [
        "AcDbBlockRepresentation",
        "AppDataCache",
        "ACAD_ENHANCEDBLOCKDATA",
    ] {
        owner = ensure_dictionary(doc, owner, key)?;
    }
    let key = p.id.to_string();
    let h = if let Some(h) = entry(doc, owner, &key)? {
        h
    } else {
        let h = doc.allocate_handle();
        let mut r = XRecord::named(&key);
        r.handle = h;
        r.owner = owner;
        r.reactors.push(owner);
        r.cloning_flags = codec::objects::DictionaryCloningFlags::KeepExisting;
        r.entries = vec![
            XRecordEntry::int32(1071, 0),
            XRecordEntry::int32(1071, 0),
            XRecordEntry::int16(70, 25),
            XRecordEntry::int16(70, 104),
        ];
        let points = if p.point {
            vec![p.end]
        } else if p.xy {
            vec![
                DVec3::new(p.base.x, p.end.y, p.base.z),
                DVec3::new(p.end.x, p.base.y, p.end.z),
                -DVec3::Z,
            ]
        } else if p.rotation {
            vec![p.base, p.end, -DVec3::Z, p.reference]
        } else if !p.linear && !p.flip {
            vec![p.base, p.end, -DVec3::Z, p.end]
        } else {
            vec![p.base, p.end, -DVec3::Z]
        };
        for v in points {
            r.entries
                .push(XRecordEntry::new(10, XRecordValue::Point3D(v.x, v.y, v.z)));
        }
        if p.flip {
            r.entries.push(XRecordEntry::int16(70, 0));
        }
        doc.objects.insert(h, ObjectType::XRecord(r));
        if let Some(ObjectType::Dictionary(d)) = doc.objects.get_mut(&owner) {
            d.add_entry(&key, h);
        }
        h
    };
    let Some(ObjectType::XRecord(r)) = doc.objects.get_mut(&h) else {
        return Err("Invalid parameter record.");
    };
    if let Value::Flip(state) = value {
        let entry = r
            .entries
            .iter_mut()
            .skip(4)
            .rev()
            .find(|e| matches!(e.value, XRecordValue::Int16(_)))
            .ok_or("The Flip cache has no state.")?;
        entry.value = XRecordValue::Int16(i16::from(state));
        r.raw_data.clear();
        r.raw_dwg_data = None;
        return Ok(());
    }
    let Value::Point(position) = value else {
        unreachable!()
    };
    if p.xy {
        let mut points = r
            .entries
            .iter_mut()
            .filter(|e| matches!(e.value, XRecordValue::Point3D(..)));
        let a = points
            .next()
            .ok_or("The XY parameter cache has no base point.")?;
        a.value = XRecordValue::Point3D(p.base.x, position.y, p.base.z);
        let b = points
            .next()
            .ok_or("The XY parameter cache has no endpoint.")?;
        b.value = XRecordValue::Point3D(position.x, p.base.y, position.z);
    } else {
        let index = usize::from(!p.point);
        let e = r
            .entries
            .iter_mut()
            .filter(|e| matches!(e.value, XRecordValue::Point3D(..)))
            .nth(index)
            .ok_or("The reference parameter cache has no endpoint.")?;
        e.value = XRecordValue::Point3D(position.x, position.y, position.z);
    }
    // Retain unknown metadata and references, but invalidate stale raw bytes.
    r.raw_data.clear();
    r.raw_dwg_data = None;
    Ok(())
}

pub(crate) fn definition(doc: &CadDocument, insert: Handle) -> Option<Handle> {
    let EntityType::Insert(i) = doc.get_entity(insert)? else {
        return None;
    };
    let s = i.x_scale().abs();
    if s < 1e-12
        || (i.y_scale().abs() - s).abs() > 1e-10 * s.max(1.)
        || (i.z_scale().abs() - s).abs() > 1e-10 * s.max(1.)
    {
        return None;
    }
    doc.dynamic_definition_for_insert(insert)
}

/// Stage and validate completely before exposing any changes to the drawing.
#[cfg(test)]
pub(crate) fn evaluate(
    doc: &CadDocument,
    insert: Handle,
    parameter: Handle,
    local: DVec3,
) -> Result<Option<CadDocument>, &'static str> {
    evaluate_value(doc, insert, parameter, Value::Point(local))
}

pub(crate) fn evaluate_value(
    doc: &CadDocument,
    insert: Handle,
    parameter: Handle,
    input: Value,
) -> Result<Option<CadDocument>, &'static str> {
    let def =
        definition(doc, insert).ok_or("Select a uniformly scaled dynamic block reference.")?;
    let parameters = all_parameters(doc, def);
    let selected = parameters
        .iter()
        .find(|p| p.handle == parameter)
        .ok_or("The reference parameter is unavailable.")?;
    let mut evaluated = doc.clone();
    for p in &parameters {
        let value = value(doc, insert, p)?;
        super::set_value(&mut evaluated, p, value)?;
    }
    if !super::evaluate_value(&mut evaluated, doc, def, parameter, input)? {
        return Ok(None);
    }
    let value = super::value(&evaluated, selected)?;
    let mut result = materialize(doc, insert, def, &evaluated)?;
    save(&mut result, insert, selected, value)?;
    Ok(Some(result))
}

/// Refresh existing evaluated references after an authoring change.
pub(crate) fn refresh(doc: &CadDocument, def: Handle) -> Result<CadDocument, &'static str> {
    let mut staged = doc.clone();
    let inserts = doc
        .entities()
        .filter_map(|e| match e {
            EntityType::Insert(i)
                if doc.dynamic_definition_for_insert(i.common.handle) == Some(def)
                    && doc
                        .block_records
                        .get(&i.block_name)
                        .is_some_and(|b| b.handle != def) =>
            {
                Some(i.common.handle)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for insert in inserts {
        let mut evaluated = staged.clone();
        for p in all_parameters(&staged, def) {
            let v = value(&staged, insert, &p)?;
            super::set_value(&mut evaluated, &p, v)?;
        }
        super::rebuild(&mut evaluated, &staged, def)?;
        staged = materialize(&staged, insert, def, &evaluated)?;
    }
    Ok(staged)
}

fn materialize(
    doc: &CadDocument,
    insert: Handle,
    def: Handle,
    evaluated: &CadDocument,
) -> Result<CadDocument, &'static str> {
    let mut scene = crate::scene::Scene::new();
    scene.document = doc.clone();
    scene.dynamic_representation(insert);
    let Some(EntityType::Insert(reference)) = scene.document.get_entity(insert) else {
        return Err("Missing block reference.");
    };
    let name = reference.block_name.clone();
    if scene
        .document
        .block_records
        .get(&name)
        .is_none_or(|b| b.handle == def)
    {
        return Err("Could not isolate the dynamic block reference.");
    }
    // Rebuild all members so removed targets lose their old action effects.
    // Preserve each representation member's identity and visibility.
    let source = doc
        .block_records
        .iter()
        .find(|b| b.handle == def)
        .ok_or("Missing definition.")?;
    let target = scene.document.block_records.get(&name).unwrap();
    if source.entity_handles.len() != target.entity_handles.len() {
        return Err("The reference topology no longer matches its definition.");
    }
    let pairs = source
        .entity_handles
        .iter()
        .copied()
        .zip(target.entity_handles.iter().copied())
        .collect::<Vec<_>>();
    for (from, to) in pairs {
        let mut e = evaluated
            .get_entity(from)
            .cloned()
            .ok_or("Missing action target.")?;
        let target = scene
            .document
            .get_entity_mut(to)
            .ok_or("Missing representation target.")?;
        if std::mem::discriminant(&e) != std::mem::discriminant(target) {
            return Err("The reference topology no longer matches its definition.");
        }
        *e.common_mut() = target.common().clone();
        *target = e;
    }
    Ok(scene.document)
}
