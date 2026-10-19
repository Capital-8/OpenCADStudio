// 3D Studio import: what one import reads and how.

use std::collections::HashMap;

use crate::io::tds_import::{LayerMode, MultiMaterial};

/// What one 3D Studio import reads and how.
#[derive(Clone, Debug, PartialEq)]
pub struct TdsImportRequest {
    pub path: String,
    /// The objects to import, by name ("Global Settings" included).
    pub selected: Vec<String>,
    pub layers: LayerMode,
    /// The choice for each mesh with several materials, by name.
    pub multi: HashMap<String, MultiMaterial>,
    /// Meshes keep only the vertices their faces use (the import splits or
    /// asks), else the file's vertex list.
    pub compact: bool,
    /// The meshes with several materials were asked about one by one: the
    /// materials then stay out of the objects and are linked through their
    /// legacy render records only.
    pub asked: bool,
    /// The mesh whose question was cancelled: the import stops there.
    pub stop_at: Option<String>,
}
