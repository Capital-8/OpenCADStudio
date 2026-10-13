//! Reference-only designation (API v9).
//!
//! A reference-only entity is plugin-owned display geometry: it renders and
//! snaps normally, but the built-in edit paths treat it like an entity on a
//! locked layer — no grips, no click picking (a click passes through to the
//! editable entity underneath), and user-driven writes refuse it. The owning
//! plugin keeps full write access through the plugin host API, so it can
//! refresh derived geometry in place.
//!
//! The designation persists as a marker value inside an XDATA record under a
//! host-owned APPID, so it round-trips DWG and DXF with the entity and never
//! collides with plugin APPID namespaces (plugins register their own
//! application names through their manifest).

use codec::entities::EntityType;
use codec::xdata::{ExtendedDataRecord, XDataValue};

/// Host-owned APPID carrying the designation. The name belongs to the host
/// alone; plugin records live under the plugin's own registered APPIDs.
pub const REFERENCE_ONLY_APP_ID: &str = "OCS";

/// Marker value (DXF group code 1000) that flags an entity reference-only.
/// Human-readable so an exported file explains itself.
const REFERENCE_ONLY_MARKER: &str = "REFERENCE_ONLY";

/// True when `value` is the reference-only marker.
fn is_marker(value: &XDataValue) -> bool {
    matches!(value, XDataValue::String(text) if text.as_str() == REFERENCE_ONLY_MARKER)
}

/// True when `entity` carries the reference-only designation. Tolerant read:
/// a record without the marker (foreign or damaged content) is not
/// reference-only.
pub fn entity_is_reference_only(entity: &EntityType) -> bool {
    let Some(record) = entity
        .common()
        .extended_data
        .get_record(REFERENCE_ONLY_APP_ID)
    else {
        return false;
    };
    record.values.iter().any(is_marker)
}

/// Set (or clear) the designation on `entity`, returning whether the entity
/// changed. Only the marker value is added or removed; foreign values under
/// the host APPID are preserved verbatim, and a record left without values is
/// dropped so a cleared flag leaves no empty record behind.
pub fn set_entity_reference_only(entity: &mut EntityType, reference_only: bool) -> bool {
    if entity_is_reference_only(entity) == reference_only {
        return false;
    }
    let xd = &mut entity.common_mut().extended_data;
    if reference_only {
        let mut record = xd
            .get_record(REFERENCE_ONLY_APP_ID)
            .cloned()
            .unwrap_or_else(|| ExtendedDataRecord::new(REFERENCE_ONLY_APP_ID));
        record
            .values
            .push(XDataValue::String(REFERENCE_ONLY_MARKER.to_owned()));
        xd.upsert_record(record);
    } else {
        let Some(mut record) = xd.get_record(REFERENCE_ONLY_APP_ID).cloned() else {
            return false;
        };
        record.values.retain(|value| !is_marker(value));
        if record.values.is_empty() {
            xd.remove_record(REFERENCE_ONLY_APP_ID);
        } else {
            xd.upsert_record(record);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec::entities::Line;

    fn entity() -> EntityType {
        EntityType::Line(Line::new())
    }

    #[test]
    fn marking_flags_and_unmarking_clears() {
        let mut entity = entity();
        assert!(!entity_is_reference_only(&entity));
        assert!(set_entity_reference_only(&mut entity, true));
        assert!(entity_is_reference_only(&entity));
        // Idempotent: a second mark changes nothing and reports it.
        assert!(!set_entity_reference_only(&mut entity, true));
        assert!(set_entity_reference_only(&mut entity, false));
        assert!(!entity_is_reference_only(&entity));
        assert!(!set_entity_reference_only(&mut entity, false));
        // A cleared flag leaves no empty record behind.
        assert!(entity
            .common()
            .extended_data
            .get_record(REFERENCE_ONLY_APP_ID)
            .is_none());
    }

    #[test]
    fn foreign_values_under_the_host_app_id_survive() {
        let mut entity = entity();
        let mut record = ExtendedDataRecord::new(REFERENCE_ONLY_APP_ID);
        record.values.push(XDataValue::Integer16(7));
        entity.common_mut().extended_data.add_record(record);
        // A record without the marker is not reference-only.
        assert!(!entity_is_reference_only(&entity));
        assert!(set_entity_reference_only(&mut entity, true));
        let record = entity
            .common()
            .extended_data
            .get_record(REFERENCE_ONLY_APP_ID)
            .expect("record exists");
        assert_eq!(
            record.values,
            vec![
                XDataValue::Integer16(7),
                XDataValue::String(REFERENCE_ONLY_MARKER.to_owned()),
            ]
        );
        assert!(set_entity_reference_only(&mut entity, false));
        let record = entity
            .common()
            .extended_data
            .get_record(REFERENCE_ONLY_APP_ID)
            .expect("record survives");
        assert_eq!(record.values, vec![XDataValue::Integer16(7)]);
    }

    #[test]
    fn other_apps_are_untouched() {
        let mut entity = entity();
        assert!(set_entity_reference_only(&mut entity, true));
        let mut foreign = ExtendedDataRecord::new("CVIFC");
        foreign.values.push(XDataValue::String("road".to_owned()));
        entity.common_mut().extended_data.add_record(foreign);
        assert!(set_entity_reference_only(&mut entity, false));
        assert!(entity
            .common()
            .extended_data
            .get_record("CVIFC")
            .is_some_and(|record| record.values.len() == 1));
    }
}
