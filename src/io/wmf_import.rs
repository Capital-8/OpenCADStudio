//! Windows metafile (WMF) reading: the records a metafile plays, as the
//! drawing objects of a block — outlines as polylines in their pen's colour
//! and width, filled shapes as closed polylines in their brush's colour,
//! ellipses as 99-segment polylines, text as single-line text.
//!
//! The picture is scaled to fit half the view it is imported into, with its
//! top-left corner at the block's origin and y running down the screen as
//! a metafile's does.

use codec::entities::{Polyline2D, Text, Vertex2D};
use codec::types::{Color, Vector3};
use codec::EntityType;

/// A picture: its objects, in block coordinates, and the font of each
/// text (an index into `fonts`).
pub struct Picture {
    pub entities: Vec<EntityType>,
    /// The face name of each font the text uses; text styles are
    /// `WMF-<n>` for each, in this order.
    pub fonts: Vec<String>,
    /// For each text in `entities` (by position), its font index.
    pub text_fonts: Vec<(usize, usize)>,
}

#[derive(Clone, Copy)]
struct Pen {
    null: bool,
    width: f64,
    rgb: [u8; 3],
}

#[derive(Clone, Copy)]
struct Brush {
    null: bool,
    rgb: [u8; 3],
}

#[derive(Clone)]
struct Font {
    height: f64,
    escapement: f64,
    face: String,
}

#[derive(Clone)]
enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(Font),
    Other,
}

/// Segments of a whole ellipse.
const ELLIPSE_SEGMENTS: usize = 99;

/// A colour reference as the nearest index colour; black and white both
/// draw as colour 7.
fn color(rgb: [u8; 3]) -> Color {
    if rgb == [0, 0, 0] || rgb == [255, 255, 255] {
        return Color::Index(7);
    }
    Color::Index(codec::types::nearest_aci(rgb[0], rgb[1], rgb[2]))
}

fn colorref(lo: u16, hi: u16) -> [u8; 3] {
    [(lo & 0xff) as u8, (lo >> 8) as u8, (hi & 0xff) as u8]
}

struct Player {
    origin: [f64; 2],
    scale: [f64; 2],
    entities: Vec<EntityType>,
    fonts: Vec<String>,
    text_fonts: Vec<(usize, usize)>,
}

impl Player {
    fn at(&self, x: f64, y: f64) -> Vector3 {
        Vector3::new((x - self.origin[0]) * self.scale[0], (y - self.origin[1]) * self.scale[1], 0.0)
    }

    fn polyline(&mut self, points: &[[f64; 2]], closed: bool, width: f64, rgb: [u8; 3]) {
        if points.len() < 2 {
            return;
        }
        let width = width * self.scale[0].abs();
        let mut polyline = Polyline2D::new();
        polyline.start_width = width;
        polyline.end_width = width;
        for p in points {
            let mut vertex = Vertex2D::new(self.at(p[0], p[1]));
            vertex.start_width = width;
            vertex.end_width = width;
            polyline.add_vertex(vertex);
        }
        if closed {
            polyline.close();
        }
        polyline.common.color = color(rgb);
        self.entities.push(EntityType::Polyline2D(polyline));
    }

    /// An outline in the pen, or a filled area as its brush's outline.
    fn shape(&mut self, points: &[[f64; 2]], pen: &Pen, brush: &Brush) {
        let width = if pen.null { 0.0 } else { pen.width };
        if !brush.null {
            self.polyline(points, true, width, brush.rgb);
        } else if !pen.null {
            self.polyline(points, true, width, pen.rgb);
        }
    }
}

/// Points round an ellipse inside `(left, top)–(right, bottom)`, from
/// `start` to `end` (radians, counter-clockwise in drawing coordinates).
fn ellipse_points(bounds: [f64; 4], start: f64, sweep: f64) -> Vec<[f64; 2]> {
    let [left, top, right, bottom] = bounds;
    let (cx, cy) = (0.5 * (left + right), 0.5 * (top + bottom));
    let (rx, ry) = (0.5 * (right - left).abs(), 0.5 * (bottom - top).abs());
    let step = std::f64::consts::TAU / ELLIPSE_SEGMENTS as f64;
    let count = ((sweep.abs() / step).round() as usize).max(1);
    (0..=count)
        .map(|k| {
            let a = start + sweep * k as f64 / count as f64;
            // Metafile y runs down: counter-clockwise on the drawing is
            // up the page.
            [cx + rx * a.cos(), cy - ry * a.sin()]
        })
        .collect()
}

/// The angle of a radial point round an ellipse's centre, in drawing
/// orientation.
fn angle(bounds: [f64; 4], x: f64, y: f64) -> f64 {
    let [left, top, right, bottom] = bounds;
    let (cx, cy) = (0.5 * (left + right), 0.5 * (top + bottom));
    (-(y - cy)).atan2(x - cx)
}

/// Reads a metafile, scaled so the picture fits within `fit` (width and
/// height, drawing units). `None` when it is not a metafile.
pub fn read(bytes: &[u8], fit: [f64; 2]) -> Option<Picture> {
    let word = |at: usize| -> Option<u16> {
        Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
    };
    let mut at = 0;
    let mut bounds = None;
    if word(0)? == 0xCDD7 && word(2)? == 0x9AC6 {
        let s = |k: usize| word(6 + 2 * k).map(|w| w as i16 as f64);
        bounds = Some([s(0)?, s(1)?, s(2)?, s(3)?]);
        at = 22;
    }
    // The metafile header: type 1 or 2, nine words long.
    if !matches!(word(at)?, 1 | 2) || word(at + 2)? != 9 {
        return None;
    }
    at += 18;
    let mut records = Vec::new();
    while at + 6 <= bytes.len() {
        let size = (u32::from(word(at)?) | (u32::from(word(at + 2)?) << 16)) as usize;
        let function = word(at + 4)?;
        if function == 0 || size < 3 {
            break;
        }
        let params: Vec<u16> = (0..size - 3).filter_map(|k| word(at + 6 + 2 * k)).collect();
        records.push((function, params));
        at += 2 * size;
    }

    // The window first: where the picture is and how big.
    let mut origin = [0.0, 0.0];
    let mut extent = None;
    for (function, p) in &records {
        let s = |k: usize| p.get(k).map(|w| *w as i16 as f64);
        match function {
            0x020B => origin = [s(1)?, s(0)?],
            0x020C => extent = Some([s(1)?, s(0)?]),
            _ => {}
        }
    }
    let extent = extent
        .or_else(|| bounds.map(|[l, t, r, b]| [r - l, b - t]))
        .filter(|[w, h]| *w != 0.0 && *h != 0.0)?;
    if let (None, Some([l, t, _, _])) = (records.iter().find(|(f, _)| *f == 0x020B), bounds) {
        origin = [l, t];
    }
    let scale = (fit[0] / extent[0].abs()).min(fit[1] / extent[1].abs());
    let mut player = Player {
        origin,
        scale: [scale * extent[0].signum(), -scale * extent[1].signum()],
        entities: Vec::new(),
        fonts: Vec::new(),
        text_fonts: Vec::new(),
    };

    // A fresh device context: a one-unit black pen, a white brush.
    let mut pen = Pen {
        null: false,
        width: 1.0,
        rgb: [0, 0, 0],
    };
    let mut brush = Brush {
        null: false,
        rgb: [255, 255, 255],
    };
    let mut font = Font {
        height: 12.0,
        escapement: 0.0,
        face: String::new(),
    };
    let mut text_rgb = [0, 0, 0];
    let mut table: Vec<Option<Object>> = Vec::new();
    let mut current = [0.0, 0.0];
    for (function, p) in &records {
        let s = |k: usize| p.get(k).map(|w| *w as i16 as f64).unwrap_or(0.0);
        let points = |from: usize, count: usize| -> Vec<[f64; 2]> {
            (0..count).map(|k| [s(from + 2 * k), s(from + 2 * k + 1)]).collect()
        };
        let create = |table: &mut Vec<Option<Object>>, object: Object| {
            match table.iter().position(Option::is_none) {
                Some(slot) => table[slot] = Some(object),
                None => table.push(Some(object)),
            }
        };
        match function {
            // CREATEPENINDIRECT
            0x02FA => create(
                &mut table,
                Object::Pen(Pen {
                    null: p.first().is_some_and(|style| style & 0x0f == 5),
                    width: s(1),
                    rgb: colorref(p.get(3).copied().unwrap_or(0), p.get(4).copied().unwrap_or(0)),
                }),
            ),
            // CREATEBRUSHINDIRECT
            0x02FC => create(
                &mut table,
                Object::Brush(Brush {
                    null: p.first() == Some(&1),
                    rgb: colorref(p.get(1).copied().unwrap_or(0), p.get(2).copied().unwrap_or(0)),
                }),
            ),
            // CREATEFONTINDIRECT: height, width, escapement, ... face at
            // byte 18.
            0x02FB => {
                let mut face = Vec::new();
                for w in p.iter().skip(9) {
                    face.extend(w.to_le_bytes());
                }
                let end = face.iter().position(|b| *b == 0).unwrap_or(face.len());
                create(
                    &mut table,
                    Object::Font(Font {
                        height: s(0).abs(),
                        escapement: s(2) / 10.0,
                        face: String::from_utf8_lossy(&face[..end]).into_owned(),
                    }),
                )
            }
            // Palettes and regions take a slot too.
            0x00F7 | 0x06FF | 0x01F9 | 0x0142 => create(&mut table, Object::Other),
            // SELECTOBJECT
            0x012D => match table.get(p.first().copied().unwrap_or(0) as usize) {
                Some(Some(Object::Pen(x))) => pen = *x,
                Some(Some(Object::Brush(x))) => brush = *x,
                Some(Some(Object::Font(x))) => font = x.clone(),
                _ => {}
            },
            // DELETEOBJECT
            0x01F0 => {
                if let Some(slot) = table.get_mut(p.first().copied().unwrap_or(0) as usize) {
                    *slot = None;
                }
            }
            // SETTEXTCOLOR
            0x0209 => text_rgb = colorref(p.first().copied().unwrap_or(0), p.get(1).copied().unwrap_or(0)),
            // MOVETO, LINETO (y, x)
            0x0214 => current = [s(1), s(0)],
            0x0213 => {
                let to = [s(1), s(0)];
                if !pen.null {
                    player.polyline(&[current, to], false, pen.width, pen.rgb);
                }
                current = to;
            }
            // POLYLINE
            0x0325 => {
                if !pen.null {
                    let line = points(1, s(0) as usize);
                    player.polyline(&line, false, pen.width, pen.rgb);
                }
            }
            // POLYGON
            0x0324 => {
                let ring = points(1, s(0) as usize);
                player.shape(&ring, &pen, &brush);
            }
            // POLYPOLYGON: counts, then every ring's points
            0x0538 => {
                let rings = s(0) as usize;
                let mut from = 1 + rings;
                for k in 0..rings {
                    let count = s(1 + k) as usize;
                    let ring = points(from, count);
                    player.shape(&ring, &pen, &brush);
                    from += 2 * count;
                }
            }
            // RECTANGLE, ROUNDRECT (bottom, right, top, left; corner
            // ellipse first for the latter)
            0x041B | 0x061C => {
                let o = if *function == 0x061C { 2 } else { 0 };
                let (b, r, t, l) = (s(o), s(o + 1), s(o + 2), s(o + 3));
                player.shape(&[[l, t], [r, t], [r, b], [l, b]], &pen, &brush);
            }
            // ELLIPSE: its outline in the pen
            0x0418 => {
                if !pen.null {
                    let bounds = [s(3), s(2), s(1), s(0)];
                    let ring = ellipse_points(bounds, 0.0, std::f64::consts::TAU);
                    player.polyline(&ring, false, pen.width, pen.rgb);
                }
            }
            // ARC, CHORD, PIE: (yEnd, xEnd, yStart, xStart, bottom,
            // right, top, left), drawn counter-clockwise
            0x0817 | 0x0830 | 0x081A => {
                let bounds = [s(7), s(6), s(5), s(4)];
                let start = angle(bounds, s(3), s(2));
                let end = angle(bounds, s(1), s(0));
                let mut sweep = end - start;
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                let mut ring = ellipse_points(bounds, start, sweep);
                match function {
                    0x0817 => {
                        if !pen.null {
                            player.polyline(&ring, false, pen.width, pen.rgb);
                        }
                    }
                    _ => {
                        if *function == 0x081A {
                            ring.push([0.5 * (bounds[0] + bounds[2]), 0.5 * (bounds[1] + bounds[3])]);
                        }
                        player.shape(&ring, &pen, &brush);
                    }
                }
            }
            // TEXTOUT (count, string, y, x) and EXTTEXTOUT (y, x, count,
            // options, [rectangle], string)
            0x0521 | 0x0A32 => {
                let (count, text_at, (x, y)) = if *function == 0x0521 {
                    let count = s(0) as usize;
                    let words = count.div_ceil(2);
                    (count, 1, (s(2 + words), s(1 + words)))
                } else {
                    let count = s(2) as usize;
                    let clipped = p.get(3).is_some_and(|o| o & 0x0006 != 0);
                    (count, if clipped { 8 } else { 4 }, (s(1), s(0)))
                };
                let mut raw = Vec::new();
                for w in p.iter().skip(text_at) {
                    raw.extend(w.to_le_bytes());
                }
                raw.truncate(count);
                let value: String = raw.iter().map(|b| char::from(*b)).collect();
                if value.trim().is_empty() {
                    continue;
                }
                // The reference point is the cell's top; the baseline sits
                // nine tenths of the font height below it.
                let rotation = font.escapement.to_radians();
                let drop = 0.9 * font.height;
                let base = player.at(x + drop * rotation.sin(), y + drop * rotation.cos());
                let index = match player.fonts.iter().position(|f| *f == font.face) {
                    Some(k) => k,
                    None => {
                        player.fonts.push(font.face.clone());
                        player.fonts.len() - 1
                    }
                };
                let mut text = Text::new();
                text.value = value;
                text.insertion_point = base;
                text.height = 0.72 * font.height * scale;
                text.rotation = rotation;
                text.common.color = color(text_rgb);
                player.text_fonts.push((player.entities.len(), index));
                player.entities.push(EntityType::Text(text));
            }
            _ => {}
        }
    }
    Some(Picture {
        entities: player.entities,
        fonts: player.fonts,
        text_fonts: player.text_fonts,
    })
}
