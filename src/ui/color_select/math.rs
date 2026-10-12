//! Colour maths and text parsing for the Select Color panel.
//!
//! Everything here is pure so it can be unit tested without a UI: HSB/RGB
//! conversion, hex formatting, nearest-index lookup and the parser behind the
//! panel's Color field.

use codec::types::Color as AcadColor;

/// Hue in degrees (0–360), saturation and brightness in 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsb {
    pub h: f32,
    pub s: f32,
    pub b: f32,
}

impl Hsb {
    pub fn to_rgb(self) -> (u8, u8, u8) {
        hsb_to_rgb(self.h, self.s, self.b)
    }
}

/// Convert hue (degrees), saturation and brightness (0–1) to 8-bit RGB.
pub fn hsb_to_rgb(h: f32, s: f32, b: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let b = b.clamp(0.0, 1.0);
    let channel = |n: f32| {
        let k = (n + h / 60.0) % 6.0;
        let v = b - b * s * k.min(4.0 - k).clamp(0.0, 1.0);
        (v * 255.0).round() as u8
    };
    (channel(5.0), channel(3.0), channel(1.0))
}

/// Convert 8-bit RGB to HSB. Grays have no hue; they report 0.
pub fn rgb_to_hsb(r: u8, g: u8, b: u8) -> Hsb {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let mut h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }
    let s = if max == 0.0 { 0.0 } else { delta / max };
    Hsb { h, s, b: max }
}

/// RGB of an ACI entry, or `None` outside 1–255.
pub fn aci_rgb(index: u8) -> Option<(u8, u8, u8)> {
    codec::types::aci_table::aci_to_rgb(index)
}

/// The RGB a concrete colour displays as. Logical colours have none.
pub fn color_rgb(color: AcadColor) -> Option<(u8, u8, u8)> {
    match color {
        AcadColor::Index(i) => aci_rgb(i),
        AcadColor::Rgb { r, g, b } => Some((r, g, b)),
        _ => None,
    }
}

/// The closest index colour to an RGB value.
pub fn nearest_aci(r: u8, g: u8, b: u8) -> u8 {
    codec::types::aci_table::nearest_aci(r, g, b)
}

/// `#RRGGBB`, upper case.
pub fn hex_string(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Parse `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB`.
pub fn parse_hex(text: &str) -> Option<(u8, u8, u8)> {
    let digits = text.trim().trim_start_matches('#');
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let expanded: String = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_string(),
        _ => return None,
    };
    let value = u32::from_str_radix(&expanded, 16).ok()?;
    Some(((value >> 16) as u8, (value >> 8) as u8, value as u8))
}

/// Parse what the user typed in the Color field.
///
/// Accepts an index (`140`, `aci 140`), a standard colour name (`red`), an
/// `r,g,b` triple, or hex (`#39BFFC`). Logical colours are not accepted: the
/// panel has no ByLayer / ByBlock entry.
pub fn parse_color_input(text: &str) -> Option<AcadColor> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }
    let named = match text.as_str() {
        "red" => Some(1),
        "yellow" => Some(2),
        "green" => Some(3),
        "cyan" => Some(4),
        "blue" => Some(5),
        "magenta" => Some(6),
        "white" | "black" => Some(7),
        _ => None,
    };
    if let Some(index) = named {
        return Some(AcadColor::Index(index));
    }
    let number = text.strip_prefix("aci").map(str::trim).unwrap_or(&text);
    if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
        return match number.parse::<u16>() {
            Ok(n @ 1..=255) => Some(AcadColor::Index(n as u8)),
            _ => None,
        };
    }
    if text.contains(',') {
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        if let [r, g, b] = parts.as_slice() {
            let (r, g, b) = (r.parse().ok()?, g.parse().ok()?, b.parse().ok()?);
            return Some(AcadColor::Rgb { r, g, b });
        }
        return None;
    }
    if text.starts_with('#') || text.len() == 6 {
        let (r, g, b) = parse_hex(&text)?;
        return Some(AcadColor::Rgb { r, g, b });
    }
    None
}

/// The text the Color field shows for a colour.
pub fn color_field_text(color: AcadColor) -> String {
    match color {
        AcadColor::Index(i) => i.to_string(),
        AcadColor::Rgb { r, g, b } => format!("{r}, {g}, {b}"),
        AcadColor::ByLayer => "ByLayer".to_string(),
        AcadColor::ByBlock => "ByBlock".to_string(),
        AcadColor::None => "None".to_string(),
    }
}

/// One-line description for the panel's info line.
pub fn describe(color: AcadColor) -> String {
    match color {
        AcadColor::Index(i) => {
            let (r, g, b) = aci_rgb(i).unwrap_or((128, 128, 128));
            format!("ACI {i} · RGB {r}, {g}, {b}")
        }
        AcadColor::Rgb { r, g, b } => format!("RGB {r}, {g}, {b} · {}", hex_string(r, g, b)),
        other => color_field_text(other),
    }
}

/// Black or white text, whichever reads better on the given background.
pub fn ink_is_dark(r: u8, g: u8, b: u8) -> bool {
    0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 150.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsb_round_trips_primary_and_mixed_colours() {
        for (r, g, b) in [
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (57, 191, 252),
            (120, 200, 80),
            (128, 128, 128),
            (0, 0, 0),
            (255, 255, 255),
        ] {
            let hsb = rgb_to_hsb(r, g, b);
            assert_eq!(hsb.to_rgb(), (r, g, b), "round trip of {r},{g},{b}");
        }
    }

    #[test]
    fn hue_of_pure_colours() {
        assert_eq!(rgb_to_hsb(255, 0, 0).h, 0.0);
        assert!((rgb_to_hsb(0, 255, 0).h - 120.0).abs() < 0.01);
        assert!((rgb_to_hsb(0, 0, 255).h - 240.0).abs() < 0.01);
        assert_eq!(rgb_to_hsb(80, 80, 80).s, 0.0);
    }

    #[test]
    fn hue_wraps_around() {
        assert_eq!(hsb_to_rgb(360.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsb_to_rgb(-120.0, 1.0, 1.0), hsb_to_rgb(240.0, 1.0, 1.0));
    }

    #[test]
    fn nearest_aci_comes_from_the_codec_table() {
        assert_eq!(nearest_aci(255, 0, 0), 1);
        assert_eq!(nearest_aci(250, 5, 5), 1);
    }

    #[test]
    fn hex_formats_and_parses() {
        assert_eq!(hex_string(57, 191, 252), "#39BFFC");
        assert_eq!(parse_hex("#39bffc"), Some((57, 191, 252)));
        assert_eq!(parse_hex("39BFFC"), Some((57, 191, 252)));
        assert_eq!(parse_hex("#3bf"), Some((0x33, 0xBB, 0xFF)));
        assert_eq!(parse_hex("#39BFF"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
    }

    #[test]
    fn colour_field_accepts_index_name_rgb_and_hex() {
        assert_eq!(parse_color_input("140"), Some(AcadColor::Index(140)));
        assert_eq!(parse_color_input(" ACI 7 "), Some(AcadColor::Index(7)));
        assert_eq!(parse_color_input("Red"), Some(AcadColor::Index(1)));
        assert_eq!(
            parse_color_input("57, 191,252"),
            Some(AcadColor::Rgb {
                r: 57,
                g: 191,
                b: 252
            })
        );
        assert_eq!(
            parse_color_input("#39BFFC"),
            Some(AcadColor::Rgb {
                r: 57,
                g: 191,
                b: 252
            })
        );
    }

    #[test]
    fn colour_field_rejects_logical_and_out_of_range_values() {
        assert_eq!(parse_color_input("bylayer"), None);
        assert_eq!(parse_color_input("byblock"), None);
        assert_eq!(parse_color_input("0"), None);
        assert_eq!(parse_color_input("256"), None);
        assert_eq!(parse_color_input("300,0,0"), None);
        assert_eq!(parse_color_input("1,2"), None);
        assert_eq!(parse_color_input(""), None);
    }

    #[test]
    fn field_text_matches_parser() {
        for color in [
            AcadColor::Index(140),
            AcadColor::Rgb {
                r: 57,
                g: 191,
                b: 252,
            },
        ] {
            assert_eq!(parse_color_input(&color_field_text(color)), Some(color));
        }
    }
}
