//! Model files IMPORT reads: ACIS text (SAT), STEP and IGES, as the drawing
//! objects they become.

use codec::entities::{Entity as _, Insert};
use codec::types::{Color, Matrix4, Transform, Vector3};
use codec::EntityType;

use crate::scene::convert::acis_import::{body_entity, sat_entities};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFormat {
    Sat,
    Step,
    Iges,
}

/// The objects a file brings in.
pub struct Imported {
    /// Block definitions, each before any block that inserts it.
    pub blocks: Vec<(String, Vec<EntityType>)>,
    /// The objects for the current space.
    pub entities: Vec<EntityType>,
}

/// File units over drawing units; one where the drawing is unitless.
fn ratio(file_mm: f64, drawing_mm: Option<f64>) -> f64 {
    drawing_mm.filter(|mm| *mm > 0.0).map_or(1.0, |mm| file_mm / mm)
}

/// A block name from a product's, with the characters names cannot hold
/// replaced.
fn block_name(name: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .map(|c| if "<>/\\\":;?*|,=`".contains(c) { '_' } else { c })
        .collect();
    if name.is_empty() { "Part".to_string() } else { name }
}

/// An insert of `block` placed by `placement` (columns: axes, origin).
fn placed_insert(block: &str, placement: &[[f64; 4]; 3]) -> EntityType {
    let mut insert = Insert::new(block, Vector3::ZERO);
    let mut m = Matrix4::identity();
    for r in 0..3 {
        m.m[r] = placement[r];
    }
    insert.apply_transform(&Transform::from_matrix(m));
    EntityType::Insert(insert)
}

/// STEP: a block per product — its solids, and inserts of its components —
/// with each top product inserted at the origin, scaled from the file's
/// units to the drawing's. The solids take their colour from the block.
fn step_blocks(model: crate::io::step_import::StepModel, scale: f64) -> Imported {
    // Unique names, in product order.
    let mut names: Vec<String> = Vec::new();
    for product in &model.products {
        let base = block_name(&product.name);
        let mut name = base.clone();
        let mut n = 1;
        while names.iter().any(|taken| taken.eq_ignore_ascii_case(&name)) {
            name = format!("{base}_{n}");
            n += 1;
        }
        names.push(name);
    }
    // Children before parents.
    let mut order = Vec::new();
    let mut state = vec![0u8; model.products.len()];
    fn visit(k: usize, model: &crate::io::step_import::StepModel, state: &mut [u8], order: &mut Vec<usize>) {
        if state[k] != 0 {
            return;
        }
        state[k] = 1;
        for (child, _) in &model.products[k].children {
            visit(*child, model, state, order);
        }
        state[k] = 2;
        order.push(k);
    }
    for &root in &model.roots {
        visit(root, &model, &mut state, &mut order);
    }
    let blocks = order
        .iter()
        .map(|&k| {
            let product = &model.products[k];
            let mut entities: Vec<EntityType> = product
                .bodies
                .iter()
                .filter_map(body_entity)
                .map(|mut entity| {
                    entity.common_mut().color = Color::ByBlock;
                    entity
                })
                .collect();
            entities.extend(
                product
                    .children
                    .iter()
                    .map(|(child, placement)| {
                        let mut insert = placed_insert(&names[*child], placement);
                        insert.common_mut().color = Color::ByBlock;
                        insert
                    }),
            );
            (names[k].clone(), entities)
        })
        .collect();
    let identity = [[scale, 0.0, 0.0, 0.0], [0.0, scale, 0.0, 0.0], [0.0, 0.0, scale, 0.0]];
    Imported {
        blocks,
        entities: model
            .roots
            .iter()
            .map(|&k| placed_insert(&names[k], &identity))
            .collect(),
    }
}

/// `None` when the file is not of `format` or holds nothing it can read.
pub fn read(format: ModelFormat, bytes: &[u8], drawing_mm: Option<f64>) -> Option<Imported> {
    let imported = match format {
        ModelFormat::Sat => Imported {
            blocks: Vec::new(),
            entities: sat_entities(&String::from_utf8_lossy(bytes))?,
        },
        ModelFormat::Step => {
            let model = crate::io::step_import::read(bytes)?;
            let scale = ratio(model.unit_mm, drawing_mm);
            step_blocks(model, scale)
        }
        ModelFormat::Iges => Imported {
            blocks: Vec::new(),
            entities: crate::io::iges_import::read(bytes, drawing_mm)?
                .bodies
                .iter()
                .filter_map(|(body, color)| {
                    let mut entity = body_entity(body)?;
                    entity.common_mut().color = *color;
                    Some(entity)
                })
                .collect(),
        },
    };
    let solids = imported
        .blocks
        .iter()
        .flat_map(|(_, entities)| entities)
        .chain(&imported.entities)
        .any(|entity| !matches!(entity, EntityType::Insert(_)));
    solids.then_some(imported)
}
