// STEP AP214 export: tessellated MeshModels as an ISO 10303-21 file.
//
// Each mesh becomes a faceted B-rep: its triangles are ADVANCED_FACEs on
// their own PLANEs, sharing vertices and edges, in a CLOSED_SHELL when every
// edge bounds two triangles (a MANIFOLD_SOLID_BREP) and an OPEN_SHELL
// otherwise (a SHELL_BASED_SURFACE_MODEL). The shapes hang off the product
// structure (PRODUCT ... SHAPE_DEFINITION_REPRESENTATION) that importers
// start from, with millimetre units.

use crate::scene::model::mesh_model::MeshModel;
use std::fmt::Write as FmtWrite;

/// Build a STEP AP214 text representation from a slice of mesh models.
///
/// Returns `None` if there are no triangles to export.
pub fn build_step(meshes: &[&MeshModel]) -> Option<String> {
    struct Tri {
        v: [usize; 3],
        n: [f64; 3],
    }

    let mut data = String::new();
    let mut next_id: usize = 1;
    let mut alloc = || {
        let id = next_id;
        next_id += 1;
        id
    };
    let mut items: Vec<usize> = Vec::new();
    let mut all_solids = true;

    for mesh in meshes {
        let verts = &mesh.verts;
        let normals = &mesh.normals;
        let idx = &mesh.indices;
        // Positions are stored as an f32 half plus a low residual so they stay
        // precise at survey coordinates; write the sum, as the renderer uses.
        let point = |i: usize| {
            let high = verts[i];
            let low = mesh.verts_low.get(i).copied().unwrap_or([0.0; 3]);
            [0, 1, 2].map(|k| high[k] as f64 + low[k] as f64)
        };
        // Faces share vertices by position: the tessellator repeats a vertex
        // per face where the normal changes, and a seam's two copies differ
        // in the last bits, but the shell is one surface. Positions weld on a
        // grid of 1e-9 of the mesh's size.
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for i in 0..verts.len() {
            let p = point(i);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let size = (hi[0] - lo[0]).hypot(hi[1] - lo[1]).hypot(hi[2] - lo[2]);
        let grid = if size.is_finite() && size > 0.0 { size * 1e-9 } else { 1.0 };
        let mut points: Vec<[f64; 3]> = Vec::new();
        let mut by_position: std::collections::HashMap<[i64; 3], usize> =
            std::collections::HashMap::new();
        let mut vertex = |p: [f64; 3]| {
            let key = [0, 1, 2].map(|k| ((p[k] - lo[k]) / grid).round() as i64);
            *by_position.entry(key).or_insert_with(|| {
                points.push(p);
                points.len() - 1
            })
        };
        let mut tris: Vec<Tri> = Vec::new();
        for t in 0..idx.len() / 3 {
            let i = [idx[t * 3], idx[t * 3 + 1], idx[t * 3 + 2]].map(|i| i as usize);
            if i.iter().any(|&i| i >= verts.len()) {
                continue;
            }
            let [a, b, c] = i.map(point);
            let v = [vertex(a), vertex(b), vertex(c)];
            if v[0] == v[1] || v[1] == v[2] || v[0] == v[2] {
                continue;
            }
            // The face is placed on the triangle's own plane. A smoothed vertex
            // normal leans off it, so only a triangle whose cross product is
            // zero falls back to the normal the mesh gives it. Small triangles
            // have a tiny cross product but still a plane of their own.
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let len = cross[0].hypot(cross[1]).hypot(cross[2]);
            let n = if len == 0.0 && i[0] < normals.len() {
                normals[i[0]].map(f64::from)
            } else {
                let len = len.max(f64::MIN_POSITIVE);
                cross.map(|value| value / len)
            };
            tris.push(Tri { v, n });
        }
        if tris.is_empty() {
            continue;
        }

        let point_ids: Vec<usize> = points
            .iter()
            .map(|&[x, y, z]| {
                let cp = alloc();
                let vp = alloc();
                writeln!(data, "#{cp} = CARTESIAN_POINT('',({x:.6},{y:.6},{z:.6}));").ok();
                writeln!(data, "#{vp} = VERTEX_POINT('',#{cp});").ok();
                cp
            })
            .collect();
        // VERTEX_POINT ids follow their CARTESIAN_POINT.
        let vertex_id = |v: usize| point_ids[v] + 1;

        // One EDGE_CURVE per vertex pair, oriented from the lower index, and
        // how many triangles use it.
        let mut edges: std::collections::HashMap<(usize, usize), (usize, usize)> =
            std::collections::HashMap::new();
        let mut faces = Vec::with_capacity(tris.len());
        for tri in &tris {
            let mut oriented = [0; 3];
            for k in 0..3 {
                let (from, to) = (tri.v[k], tri.v[(k + 1) % 3]);
                let key = (from.min(to), from.max(to));
                let curve = match edges.get_mut(&key) {
                    Some(entry) => {
                        entry.1 += 1;
                        entry.0
                    }
                    None => {
                        let (start, end) = key;
                        let (p, q) = (points[start], points[end]);
                        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                        let length = d[0].hypot(d[1]).hypot(d[2]).max(f64::EPSILON);
                        let (dir, vector, line, curve) = (alloc(), alloc(), alloc(), alloc());
                        writeln!(
                            data,
                            "#{dir} = DIRECTION('',({:.6},{:.6},{:.6}));",
                            d[0] / length,
                            d[1] / length,
                            d[2] / length
                        )
                        .ok();
                        writeln!(data, "#{vector} = VECTOR('',#{dir},{length:.6});").ok();
                        writeln!(data, "#{line} = LINE('',#{},#{vector});", point_ids[start]).ok();
                        writeln!(
                            data,
                            "#{curve} = EDGE_CURVE('',#{},#{},#{line},.T.);",
                            vertex_id(start),
                            vertex_id(end)
                        )
                        .ok();
                        edges.insert(key, (curve, 1));
                        curve
                    }
                };
                let id = alloc();
                let sense = if from == key.0 { ".T." } else { ".F." };
                writeln!(data, "#{id} = ORIENTED_EDGE('',*,*,#{curve},{sense});").ok();
                oriented[k] = id;
            }
            let (lp, bound, normal, reference, placement, plane, face) =
                (alloc(), alloc(), alloc(), alloc(), alloc(), alloc(), alloc());
            writeln!(
                data,
                "#{lp} = EDGE_LOOP('',(#{},#{},#{}));",
                oriented[0], oriented[1], oriented[2]
            )
            .ok();
            writeln!(data, "#{bound} = FACE_OUTER_BOUND('',#{lp},.T.);").ok();
            let [nx, ny, nz] = tri.n;
            writeln!(data, "#{normal} = DIRECTION('',({nx:.6},{ny:.6},{nz:.6}));").ok();
            // The reference direction: the first edge, made square to the
            // normal (a degenerate triangle's edge need not be).
            let (p, q) = (points[tri.v[0]], points[tri.v[1]]);
            let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
            let along = d[0] * nx + d[1] * ny + d[2] * nz;
            let r = [d[0] - along * nx, d[1] - along * ny, d[2] - along * nz];
            let r_len = r[0].hypot(r[1]).hypot(r[2]);
            let r = if r_len > 0.0 {
                r.map(|value| value / r_len)
            } else if nx.abs() < 0.9 {
                [0.0, -nz, ny]
            } else {
                [-nz, 0.0, nx]
            };
            writeln!(
                data,
                "#{reference} = DIRECTION('',({:.6},{:.6},{:.6}));",
                r[0], r[1], r[2]
            )
            .ok();
            writeln!(
                data,
                "#{placement} = AXIS2_PLACEMENT_3D('',#{},#{normal},#{reference});",
                point_ids[tri.v[0]]
            )
            .ok();
            writeln!(data, "#{plane} = PLANE('',#{placement});").ok();
            writeln!(data, "#{face} = ADVANCED_FACE('',(#{bound}),#{plane},.T.);").ok();
            faces.push(face);
        }

        let face_list = faces
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(",");
        let closed = edges.values().all(|(_, uses)| *uses == 2);
        let (shell, item) = (alloc(), alloc());
        if closed {
            writeln!(data, "#{shell} = CLOSED_SHELL('',({face_list}));").ok();
            writeln!(data, "#{item} = MANIFOLD_SOLID_BREP('',#{shell});").ok();
        } else {
            all_solids = false;
            writeln!(data, "#{shell} = OPEN_SHELL('',({face_list}));").ok();
            writeln!(data, "#{item} = SHELL_BASED_SURFACE_MODEL('',(#{shell}));").ok();
        }
        items.push(item);
    }

    if items.is_empty() {
        return None;
    }

    // ── Units, context and the product structure ─────────────────────────
    let (app, protocol, product_context, product, formation, definition_context) =
        (alloc(), alloc(), alloc(), alloc(), alloc(), alloc());
    let (definition, definition_shape, length, angle, solid_angle, uncertainty) =
        (alloc(), alloc(), alloc(), alloc(), alloc(), alloc());
    let (context, representation, shape_definition, category) =
        (alloc(), alloc(), alloc(), alloc());
    writeln!(
        data,
        "#{app} = APPLICATION_CONTEXT('core data for automotive mechanical design processes');"
    )
    .ok();
    writeln!(
        data,
        "#{protocol} = APPLICATION_PROTOCOL_DEFINITION('international standard',\
         'automotive_design',2000,#{app});"
    )
    .ok();
    writeln!(data, "#{product_context} = PRODUCT_CONTEXT('',#{app},'mechanical');").ok();
    writeln!(
        data,
        "#{product} = PRODUCT('OpenCADStudio','OpenCADStudio','',(#{product_context}));"
    )
    .ok();
    writeln!(data, "#{formation} = PRODUCT_DEFINITION_FORMATION('','',#{product});").ok();
    writeln!(
        data,
        "#{definition_context} = PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design');"
    )
    .ok();
    writeln!(
        data,
        "#{definition} = PRODUCT_DEFINITION('design','',#{formation},#{definition_context});"
    )
    .ok();
    writeln!(data, "#{definition_shape} = PRODUCT_DEFINITION_SHAPE('','',#{definition});").ok();
    writeln!(
        data,
        "#{category} = PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#{product}));"
    )
    .ok();
    writeln!(data, "#{length} = (LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));").ok();
    writeln!(data, "#{angle} = (NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));").ok();
    writeln!(
        data,
        "#{solid_angle} = (NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());"
    )
    .ok();
    writeln!(
        data,
        "#{uncertainty} = UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-06),#{length},\
         'distance_accuracy_value','confusion accuracy');"
    )
    .ok();
    writeln!(
        data,
        "#{context} = (GEOMETRIC_REPRESENTATION_CONTEXT(3)\
         GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{uncertainty}))\
         GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length},#{angle},#{solid_angle}))\
         REPRESENTATION_CONTEXT('Context3D','3D Context with 1e-06 uncertainty'));"
    )
    .ok();
    let item_list = items
        .iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",");
    // Surface models are not B-rep solids, so a mixed export stays a plain
    // shape representation.
    let kind = if all_solids {
        "ADVANCED_BREP_SHAPE_REPRESENTATION"
    } else {
        "SHAPE_REPRESENTATION"
    };
    writeln!(
        data,
        "#{representation} = {kind}('OpenCADStudio',({item_list}),#{context});"
    )
    .ok();
    writeln!(
        data,
        "#{shape_definition} = SHAPE_DEFINITION_REPRESENTATION(#{definition_shape},\
         #{representation});"
    )
    .ok();

    // ── Assemble file ─────────────────────────────────────────────────────
    let ts = chrono_timestamp();
    let file = format!(
        "ISO-10303-21;\n\
         HEADER;\n\
         FILE_DESCRIPTION(('Open CAD Studio STEP export'),'2;1');\n\
         FILE_NAME('export.step','{ts}',(''),(''),'Open CAD Studio','Open CAD Studio','');\n\
         FILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\n\
         ENDSEC;\n\
         DATA;\n\
         {data}\
         ENDSEC;\n\
         END-ISO-10303-21;\n"
    );

    Some(file)
}

/// Web: no save dialog — the export continues with a virtual filename and
/// the generated bytes are delivered as a browser download.
#[cfg(target_arch = "wasm32")]
pub async fn pick_step_path_owned(stem: String) -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(format!("{stem}.step")))
}

/// Returns an ISO 8601-like timestamp string for the STEP file header.
fn chrono_timestamp() -> String {
    // Use seconds since Unix epoch for a simple timestamp without chrono dep.
    // `SystemTime::now` panics in the browser; ask the page's clock there. (#761)
    #[cfg(target_arch = "wasm32")]
    let secs = (js_sys::Date::now() / 1000.0) as u64;
    #[cfg(not(target_arch = "wasm32"))]
    let secs = {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    };
    // Format: YYYY-MM-DDTHH:MM:SS, UTC.
    let (hh, mm, ss) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let (year, month, day) = crate::patreon::civil_from_days((secs / 86_400) as i64);
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}")
}

#[cfg(test)]
mod tests {
    use super::build_step;
    use crate::scene::model::mesh_model::MeshModel;

    fn triangle(verts: Vec<[f32; 3]>, verts_low: Vec<[f32; 3]>) -> MeshModel {
        MeshModel {
            name: String::new(),
            verts,
            verts_low,
            normals: Vec::new(),
            indices: vec![0, 1, 2],
            triangle_material_handles: Vec::new(),
            triangle_colors: Vec::new(),
            color: [1.0; 4],
            selected: false,
        }
    }

    /// Split an absolute position into the f32 pair the tessellator stores.
    fn split(point: [f64; 3]) -> ([f32; 3], [f32; 3]) {
        let high = point.map(|v| v as f32);
        let low = [0, 1, 2].map(|i| (point[i] - high[i] as f64) as f32);
        (high, low)
    }

    /// At survey coordinates the f32 half alone is 6 cm coarse: 500000.123
    /// is stored as 500000.125 plus a -0.002 residual. The export has to add
    /// the residual back, as the renderer does.
    #[test]
    fn large_coordinates_keep_their_low_residual() {
        let (verts, verts_low): (Vec<_>, Vec<_>) = [
            [500000.123, 500010.456, 0.0],
            [500001.123, 500010.456, 0.0],
            [500000.123, 500011.456, 0.0],
        ]
        .into_iter()
        .map(split)
        .unzip();
        let step = build_step(&[&triangle(verts, verts_low)]).expect("step");
        assert!(
            step.contains("CARTESIAN_POINT('',(500000.123000,500010.456000,0.000000));"),
            "first vertex lost its residual:\n{}",
            step.lines()
                .find(|line| line.contains("CARTESIAN_POINT"))
                .unwrap_or("")
        );
    }

    /// Meshes without residuals (imported OBJ, legacy meshes) export as before.
    #[test]
    fn a_mesh_without_low_residuals_exports_its_positions() {
        let mesh = triangle(
            vec![[1.5, 2.0, 0.0], [2.5, 2.0, 0.0], [1.5, 3.0, 0.0]],
            Vec::new(),
        );
        let step = build_step(&[&mesh]).expect("step");
        assert!(step.contains("CARTESIAN_POINT('',(1.500000,2.000000,0.000000));"));
    }

    /// The direction each face's PLANE is placed with, read back through its
    /// AXIS2_PLACEMENT_3D. The export writes one face per triangle.
    fn plane_normals(step: &str) -> Vec<[f64; 3]> {
        let direction = |id: &str| -> [f64; 3] {
            let prefix = format!("{id} = DIRECTION('',(");
            let line = step
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap_or_else(|| panic!("no DIRECTION {id}"));
            let values: Vec<f64> = line[prefix.len()..]
                .trim_end_matches("));")
                .split(',')
                .map(|value| value.parse().expect("number"))
                .collect();
            [values[0], values[1], values[2]]
        };
        step.lines()
            .filter_map(|line| line.split_once(" = AXIS2_PLACEMENT_3D('',"))
            .map(|(_, refs)| {
                let refs: Vec<&str> = refs.trim_end_matches(");").split(',').collect();
                direction(refs[1])
            })
            .collect()
    }

    /// A curved surface shares smoothed normals across its facets, so a
    /// vertex normal leans away from the triangle. The face's PLANE has to be
    /// the triangle's own plane, or its corners do not lie on it.
    #[test]
    fn a_face_plane_follows_the_triangle_not_its_vertex_normal() {
        let mut mesh = triangle(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            Vec::new(),
        );
        let leaning = [
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        mesh.normals = vec![leaning; 3];
        let step = build_step(&[&mesh]).expect("step");
        assert_eq!(plane_normals(&step), vec![[0.0, 0.0, 1.0]]);
    }

    /// A small triangle still has a plane of its own. Its cross product is
    /// tiny (1e-18 here), far below f64::EPSILON, but it is not zero.
    #[test]
    fn a_small_triangle_keeps_its_own_plane() {
        let mut mesh = triangle(
            vec![[0.0, 0.0, 0.0], [1e-9, 0.0, 0.0], [0.0, 1e-9, 0.0]],
            Vec::new(),
        );
        let leaning = [
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        mesh.normals = vec![leaning; 3];
        let step = build_step(&[&mesh]).expect("step");
        assert_eq!(plane_normals(&step), vec![[0.0, 0.0, 1.0]]);
    }

    /// A triangle with no area has no plane of its own, so it keeps the
    /// normal the mesh gives it rather than a zero direction.
    #[test]
    fn a_degenerate_triangle_keeps_its_vertex_normal() {
        let mut mesh = triangle(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            Vec::new(),
        );
        mesh.normals = vec![[0.0, 1.0, 0.0]; 3];
        let step = build_step(&[&mesh]).expect("step");
        assert_eq!(plane_normals(&step), vec![[0.0, 1.0, 0.0]]);
    }
}

