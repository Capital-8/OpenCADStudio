//! IGES reading: the manifold solids (type 186) of a file, their shells,
//! faces, loops and edges (514/510/508/504/502) over spline surfaces and
//! curves (128/126), lines and arcs (110/100), as kernel bodies.

use std::collections::HashMap;

use kernel::brep::{Circle3, Curve3, EdgeKey, Line3, Surface, VertexKey};
use kernel::space::{NurbsCurve3, NurbsSurface3, Plane};

use super::brep_build::{Builder, FaceSpec};

/// One parameter: a number, or a Hollerith string.
#[derive(Debug, Clone)]
enum P {
    Num(f64),
    Str(String),
}

impl P {
    fn num(&self) -> f64 {
        match self {
            P::Num(v) => *v,
            P::Str(_) => 0.0,
        }
    }
}

/// Splits a section's text into parameters at `delimiter`, up to the first
/// `terminator`, reading `nH…` strings whole.
fn parameters(text: &str, delimiter: char, terminator: char) -> Vec<P> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while chars.get(i).is_some_and(|c| *c == ' ') {
            i += 1;
        }
        let start = i;
        while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
        }
        if i > start && matches!(chars.get(i), Some('H' | 'h')) {
            let count: usize = chars[start..i].iter().collect::<String>().parse().unwrap_or(0);
            let body: String = chars.iter().skip(i + 1).take(count).collect();
            i += 1 + count;
            out.push(P::Str(body));
        } else {
            i = start;
            while chars
                .get(i)
                .is_some_and(|c| *c != delimiter && *c != terminator)
            {
                i += 1;
            }
            let raw: String = chars[start..i].iter().collect::<String>().replace(['D', 'd'], "E");
            out.push(P::Num(raw.trim().parse().unwrap_or(0.0)));
        }
        while chars.get(i).is_some_and(|c| *c == ' ') {
            i += 1;
        }
        match chars.get(i) {
            Some(c) if *c == delimiter => i += 1,
            _ => return out,
        }
    }
}

/// A directory entry and its parameters.
struct Entity {
    kind: i32,
    /// Pointer to a transformation matrix entity, or zero.
    transform: usize,
    /// Colour number, or a negated pointer to a colour definition.
    color: i64,
    params: Vec<P>,
}

struct Doc {
    /// Keyed by the entry's first directory line number.
    entities: HashMap<usize, Entity>,
    /// File units to drawing units.
    scale: f64,
}

/// A 3×4 matrix applied as `R·p + T`.
type Matrix = [[f64; 4]; 3];

impl Doc {
    fn get(&self, pointer: f64) -> Option<&Entity> {
        self.entities.get(&(pointer.abs() as usize))
    }

    /// An entry's colour: a numbered one, or a definition's RGB percentages
    /// — as the nearest standard index colour when it is one exactly.
    fn color(&self, number: i64) -> codec::types::Color {
        use codec::types::Color;
        let rgb = match number {
            1 => [0, 0, 0],
            2 => [255, 0, 0],
            3 => [0, 255, 0],
            4 => [0, 0, 255],
            5 => [255, 255, 0],
            6 => [255, 0, 255],
            7 => [0, 255, 255],
            8 => [255, 255, 255],
            n if n < 0 => match self.entities.get(&(-n as usize)).filter(|e| e.kind == 314) {
                Some(e) => [1, 2, 3].map(|k| {
                    (e.params.get(k).map_or(0.0, P::num) * 2.55).round().clamp(0.0, 255.0) as u8
                }),
                None => return Color::ByLayer,
            },
            _ => return Color::ByLayer,
        };
        match rgb {
            [255, 0, 0] => Color::Index(1),
            [255, 255, 0] => Color::Index(2),
            [0, 255, 0] => Color::Index(3),
            [0, 255, 255] => Color::Index(4),
            [0, 0, 255] => Color::Index(5),
            [255, 0, 255] => Color::Index(6),
            [255, 255, 255] => Color::Index(7),
            [r, g, b] => Color::Rgb { r, g, b },
        }
    }

    fn matrix(&self, pointer: usize) -> Option<Matrix> {
        let entity = self.entities.get(&pointer).filter(|e| e.kind == 124)?;
        let v = |k: usize| entity.params.get(k).map_or(0.0, P::num);
        let mut m = [[v(1), v(2), v(3), v(4)], [v(5), v(6), v(7), v(8)], [v(9), v(10), v(11), v(12)]];
        // Matrices chain through their own transformation pointer.
        if let Some(outer) = (entity.transform != 0).then(|| self.matrix(entity.transform)).flatten() {
            let mut product = [[0.0; 4]; 3];
            for r in 0..3 {
                for c in 0..4 {
                    product[r][c] = (0..3).map(|k| outer[r][k] * m[k][c]).sum::<f64>()
                        + if c == 3 { outer[r][3] } else { 0.0 };
                }
            }
            m = product;
        }
        Some(m)
    }

    /// `entity`'s placement applied to a point it defines.
    fn place(&self, entity: &Entity) -> impl Fn([f64; 3]) -> [f64; 3] {
        let m = (entity.transform != 0).then(|| self.matrix(entity.transform)).flatten();
        let scale = self.scale;
        move |p| {
            let p = match m {
                Some(m) => [0, 1, 2].map(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3]),
                None => p,
            };
            p.map(|c| c * scale)
        }
    }

    fn curve(&self, pointer: f64) -> Option<Curve3> {
        let entity = self.get(pointer)?;
        let p = &entity.params;
        let v = |k: usize| p.get(k).map_or(0.0, P::num);
        let place = self.place(entity);
        Some(match entity.kind {
            110 => {
                let a = place([v(1), v(2), v(3)]);
                let b = place([v(4), v(5), v(6)]);
                Curve3::Line(Line3 {
                    origin: a,
                    direction: [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                })
            }
            100 => {
                let centre = place([v(2), v(3), v(1)]);
                let x = place([v(2) + 1.0, v(3), v(1)]);
                let y = place([v(2), v(3) + 1.0, v(1)]);
                let start = place([v(4), v(5), v(1)]);
                let radius = (0..3).map(|k| (start[k] - centre[k]).powi(2)).sum::<f64>().sqrt();
                Curve3::Circle(Circle3 {
                    plane: Plane::orthonormal(
                        centre,
                        [x[0] - centre[0], x[1] - centre[1], x[2] - centre[2]],
                        {
                            let (a, b) = (
                                [x[0] - centre[0], x[1] - centre[1], x[2] - centre[2]],
                                [y[0] - centre[0], y[1] - centre[1], y[2] - centre[2]],
                            );
                            [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
                        },
                    )?,
                    radius,
                })
            }
            126 => {
                let k = v(1) as usize;
                let degree = v(2) as usize;
                let knots = (0..k + degree + 2).map(|n| v(7 + n)).collect();
                let w0 = 7 + k + degree + 2;
                let weights = (0..=k).map(|n| v(w0 + n)).collect();
                let c0 = w0 + k + 1;
                let points = (0..=k)
                    .map(|n| place([v(c0 + 3 * n), v(c0 + 3 * n + 1), v(c0 + 3 * n + 2)]))
                    .collect();
                Curve3::Nurbs(NurbsCurve3::new(degree, points, knots, Some(weights))?)
            }
            _ => return None,
        })
    }

    fn surface(&self, pointer: f64) -> Option<Surface> {
        let entity = self.get(pointer)?;
        if entity.kind != 128 {
            return None;
        }
        let v = |k: usize| entity.params.get(k).map_or(0.0, P::num);
        let place = self.place(entity);
        let (k1, k2, m1, m2) = (v(1) as usize, v(2) as usize, v(3) as usize, v(4) as usize);
        let s0 = 10;
        let t0 = s0 + k1 + m1 + 2;
        let w0 = t0 + k2 + m2 + 2;
        let c0 = w0 + (k1 + 1) * (k2 + 1);
        let u_knots = (s0..t0).map(v).collect();
        let v_knots = (t0..w0).map(v).collect();
        // Points run with the first index fastest.
        let at = |i: usize, j: usize| j * (k1 + 1) + i;
        let weights = (0..=k1)
            .map(|i| (0..=k2).map(|j| v(w0 + at(i, j))).collect())
            .collect();
        let points = (0..=k1)
            .map(|i| {
                (0..=k2)
                    .map(|j| {
                        let n = c0 + 3 * at(i, j);
                        place([v(n), v(n + 1), v(n + 2)])
                    })
                    .collect()
            })
            .collect();
        NurbsSurface3::new(m1, m2, points, u_knots, v_knots, Some(weights)).map(Surface::Nurbs)
    }
}

struct Assembly<'a> {
    doc: &'a Doc,
    build: Builder,
    vertices: HashMap<(usize, usize), VertexKey>,
    edges: HashMap<(usize, usize), EdgeKey>,
}

impl Assembly<'_> {
    fn vertex(&mut self, list: f64, index: f64) -> Option<VertexKey> {
        let key = (list.abs() as usize, index as usize);
        if let Some(&v) = self.vertices.get(&key) {
            return Some(v);
        }
        let entity = self.doc.get(list).filter(|e| e.kind == 502)?;
        let n = 2 + 3 * (key.1.checked_sub(1)?);
        let p = |k: usize| entity.params.get(k).map(P::num);
        let point = self.doc.place(entity)([p(n)?, p(n + 1)?, p(n + 2)?]);
        let v = self.build.vertex(point);
        self.vertices.insert(key, v);
        Some(v)
    }

    fn edge(&mut self, list: f64, index: f64) -> Option<EdgeKey> {
        let key = (list.abs() as usize, index as usize);
        if let Some(&e) = self.edges.get(&key) {
            return Some(e);
        }
        let entity = self.doc.get(list).filter(|e| e.kind == 504)?;
        let n = 2 + 5 * key.1.checked_sub(1)?;
        let p = |k: usize| entity.params.get(k).map(P::num);
        let curve = self.doc.curve(p(n)?)?;
        let start = self.vertex(p(n + 1)?, p(n + 2)?)?;
        let end = self.vertex(p(n + 3)?, p(n + 4)?)?;
        let e = self.build.edge(curve, start, end)?;
        self.edges.insert(key, e);
        Some(e)
    }

    fn ring(&mut self, pointer: f64) -> Option<Vec<(EdgeKey, bool)>> {
        let entity = self.doc.get(pointer).filter(|e| e.kind == 508)?;
        let p = |k: usize| entity.params.get(k).map(P::num);
        let mut ring = Vec::new();
        let mut at = 2;
        for _ in 0..p(1)? as usize {
            let (kind, list, index, agrees, pcurves) = (p(at)?, p(at + 1)?, p(at + 2)?, p(at + 3)?, p(at + 4)?);
            if kind == 0.0 {
                ring.push((self.edge(list, index)?, agrees != 0.0));
            }
            at += 5 + 2 * pcurves as usize;
        }
        Some(ring)
    }

    fn face(&mut self, pointer: f64, agrees: bool) -> Option<FaceSpec> {
        let entity = self.doc.get(pointer).filter(|e| e.kind == 510)?;
        let p = |k: usize| entity.params.get(k).map(P::num);
        let surface = self.doc.surface(p(1)?)?;
        let loops = (0..p(2)? as usize)
            .map(|n| self.ring(p(4 + n)?))
            .collect::<Option<Vec<_>>>()?;
        Some(FaceSpec {
            surface,
            forward: agrees,
            loops,
        })
    }

    fn shell(&mut self, pointer: f64, agrees: bool) -> Option<Vec<FaceSpec>> {
        let entity = self.doc.get(pointer).filter(|e| e.kind == 514)?;
        let p = |k: usize| entity.params.get(k).map(P::num);
        (0..p(1)? as usize)
            .map(|n| {
                let face_agrees = p(3 + 2 * n)? != 0.0;
                let mut face = self.face(p(2 + 2 * n)?, face_agrees == agrees)?;
                if !agrees {
                    for ring in &mut face.loops {
                        ring.reverse();
                        for (_, f) in ring.iter_mut() {
                            *f = !*f;
                        }
                    }
                }
                Some(face)
            })
            .collect()
    }
}

/// What an IGES file holds.
pub struct IgesModel {
    /// Each solid, scaled to the drawing, with its colour.
    pub bodies: Vec<(kernel::brep::Body, codec::types::Color)>,
}

/// `drawing_mm`: millimetres per drawing unit, `None` for a unitless drawing.
pub fn read(bytes: &[u8], drawing_mm: Option<f64>) -> Option<IgesModel> {
    let text = String::from_utf8_lossy(bytes);
    let mut sections: HashMap<char, Vec<&str>> = HashMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.len() < 73 {
            continue;
        }
        let Some(section) = line.get(72..73).and_then(|s| s.chars().next()) else { continue };
        sections.entry(section).or_default().push(line);
    }
    let directory = sections.get(&'D')?;
    let global: String = sections
        .get(&'G')
        .map(|lines| lines.iter().filter_map(|l| l.get(..72)).collect())
        .unwrap_or_default();
    // The delimiters are themselves the first two parameters, when given.
    let mut delimiter = ',';
    let mut terminator = ';';
    if global.starts_with("1H") {
        delimiter = global.chars().nth(2).unwrap_or(',');
        let rest = global.get(4..).unwrap_or_default();
        if rest.starts_with("1H") {
            terminator = rest.chars().nth(2).unwrap_or(';');
        }
    }
    let globals = parameters(&global, delimiter, terminator);
    let unit_mm = match globals.get(13).map(P::num).unwrap_or(1.0) as i32 {
        1 => 25.4,
        4 => 304.8,
        5 => 1_609_344.0,
        6 => 1000.0,
        7 => 1_000_000.0,
        8 => 0.0254,
        9 => 0.001,
        10 => 10.0,
        11 => 0.000_025_4,
        3 => match globals.get(14) {
            Some(P::Str(name)) if name.eq_ignore_ascii_case("IN") || name.eq_ignore_ascii_case("INCH") => 25.4,
            Some(P::Str(name)) if name.eq_ignore_ascii_case("FT") => 304.8,
            Some(P::Str(name)) if name.eq_ignore_ascii_case("M") => 1000.0,
            Some(P::Str(name)) if name.eq_ignore_ascii_case("CM") => 10.0,
            _ => 1.0,
        },
        _ => 1.0,
    } * globals.get(12).map(P::num).filter(|s| *s > 0.0).map_or(1.0, |s| 1.0 / s);

    let parameter_lines: HashMap<usize, &str> = sections
        .get(&'P')
        .map(|lines| {
            lines
                .iter()
                .filter_map(|l| Some((l.get(73..)?.trim().parse().ok()?, *l)))
                .collect()
        })
        .unwrap_or_default();
    let field = |line: &str, n: usize| -> i64 {
        line.get(n * 8..n * 8 + 8)
            .and_then(|f| f.trim().parse().ok())
            .unwrap_or(0)
    };
    let mut entities = HashMap::new();
    for pair in directory.chunks(2) {
        let [first, second] = pair else { continue };
        let Some(number) = first.get(73..).and_then(|n| n.trim().parse::<usize>().ok()) else {
            continue;
        };
        let start = field(first, 1) as usize;
        let count = field(second, 3).max(1) as usize;
        let data: String = (start..start + count)
            .filter_map(|n| parameter_lines.get(&n))
            .filter_map(|l| l.get(..64))
            .collect();
        entities.insert(
            number,
            Entity {
                kind: field(first, 0) as i32,
                transform: field(first, 6).max(0) as usize,
                color: field(second, 2),
                params: parameters(&data, delimiter, terminator),
            },
        );
    }
    let scale = drawing_mm.filter(|mm| *mm > 0.0).map_or(1.0, |mm| unit_mm / mm);
    let doc = Doc { entities, scale };

    let mut solids: Vec<usize> = doc
        .entities
        .iter()
        .filter(|(_, e)| e.kind == 186)
        .map(|(n, _)| *n)
        .collect();
    solids.sort_unstable();
    let mut bodies = Vec::new();
    for number in solids {
        let entity = &doc.entities[&number];
        let v = |k: usize| entity.params.get(k).map(P::num);
        let Some((shell, agrees)) = v(1).zip(v(2)) else { continue };
        let mut assembly = Assembly {
            doc: &doc,
            build: Builder::new(),
            vertices: HashMap::new(),
            edges: HashMap::new(),
        };
        let mut shells = Vec::new();
        let outer = assembly.shell(shell, agrees != 0.0);
        let voids = (0..v(3).unwrap_or(0.0) as usize)
            .map(|n| assembly.shell(v(4 + 2 * n)?, v(5 + 2 * n)? != 0.0))
            .collect::<Option<Vec<_>>>();
        if let (Some(outer), Some(voids)) = (outer, voids) {
            shells.push(outer);
            shells.extend(voids);
            if assembly.build.lump(shells).is_some() {
                bodies.push((assembly.build.body, doc.color(entity.color)));
            }
        }
    }
    Some(IgesModel { bodies })
}
