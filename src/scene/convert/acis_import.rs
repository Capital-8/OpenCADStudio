//! ACIS text (SAT) files and kernel bodies as drawing objects.
//!
//! Each body of a file becomes one object: a closed solid a 3DSOLID, a sheet
//! whose faces all lie on planes a REGION, any other sheet a SURFACE — the
//! classes the ACIS import of the reference application produces.

use codec::entities::acis::{SatDocument, SatFace, Sidedness};
use codec::entities::{AcisData, Region, Solid3D, Surface, SurfaceKind};
use codec::EntityType;

/// What a body restores as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Solid,
    Region,
    Surface,
}

fn class(document: &SatDocument) -> Option<Class> {
    let faces: Vec<SatFace<'_>> = document
        .records
        .iter()
        .filter_map(SatFace::from_record)
        .collect();
    if faces.is_empty() {
        return None;
    }
    if faces
        .iter()
        .all(|face| face.sidedness() != Sidedness::Double)
    {
        return Some(Class::Solid);
    }
    let planar = faces.iter().all(|face| {
        face.surface()
            .index()
            .and_then(|at| document.record(at))
            .is_some_and(|surface| surface.entity_type == "plane-surface")
    });
    Some(if planar { Class::Region } else { Class::Surface })
}

/// One body's document as its drawing object.
fn entity(document: &SatDocument) -> Option<EntityType> {
    Some(match class(document)? {
        Class::Solid => {
            let mut solid = Solid3D::new();
            solid.set_sat_document(document);
            EntityType::Solid3D(solid)
        }
        Class::Region => {
            let mut region = Region::new();
            region.set_sat_document(document);
            EntityType::Region(region)
        }
        Class::Surface => {
            let mut surface = Surface::new(SurfaceKind::Generic);
            surface.acis_data = AcisData::from_sat(&document.to_sat_string());
            EntityType::Surface(Box::new(surface))
        }
    })
}

/// The objects of an ACIS text file, one per body with faces; `None` when
/// the text is no ACIS data.
pub fn sat_entities(text: &str) -> Option<Vec<EntityType>> {
    let document = SatDocument::parse(text).ok()?;
    Some(document.split_bodies().iter().filter_map(entity).collect())
}

/// A kernel body as its drawing object, written as exact ACIS.
pub fn body_entity(body: &kernel::brep::Body) -> Option<EntityType> {
    let document = super::acis_export::solid_to_sat(body)?;
    entity(&document)
}
