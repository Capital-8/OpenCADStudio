//! Interactive front-end for the UCS command.
//!
//! Bare `UCS` enters this command so the option and its value are entered
//! step-by-step — `UCS` ⏎ → option ⏎ → value ⏎ — and so a single line
//! `UCS Z 90` (typed in the command line with Space between tokens, or sent
//! headless as `{"op":"run","cmd":"UCS Z 90"}`) feeds the option then the value
//! as text steps. Execution is delegated to the existing inline `UCS …` handler
//! via [`CmdResult::Dispatch`], so the coordinate-system math and persistence
//! stay in one place. (#169)

use crate::command::{CadCommand, CmdResult, WorkingPlane};
use crate::scene::model::wire_model::WireModel;
use crate::t;
use glam::DVec3;

#[derive(Default)]
pub struct UcsCommand {
    /// The chosen option keyword (uppercased), once entered; `None` until then.
    option: Option<String>,
    points: Vec<DVec3>,
    plane: WorkingPlane,
}

impl UcsCommand {
    pub fn new() -> Self {
        Self::default()
    }

    /// Options that take no further typed argument: they execute immediately
    /// or, for Face / OBject, start their own entity pick.
    fn is_zero_arg(opt: &str) -> bool {
        matches!(
            opt,
            "W" | "WORLD" | "VIEW" | "V" | "LIST" | "?" | "FACE" | "F" | "OBJECT" | "OB"
        )
    }

    /// Options whose argument is a coordinate (so a click is accepted too).
    fn takes_point(opt: &str) -> bool {
        matches!(opt, "ORIGIN" | "O" | "3POINT" | "3P")
    }

    /// Options that expect one more typed argument (angle or name).
    fn takes_value(opt: &str) -> bool {
        matches!(
            opt,
            "Z" | "X" | "Y" | "ORIGIN" | "O" | "SAVE" | "S" | "DELETE" | "DEL" | "D"
        )
    }
}

impl CadCommand for UcsCommand {
    fn name(&self) -> &'static str {
        "UCS"
    }

    fn prompt(&self) -> String {
        match self.option.as_deref() {
            None => {
                t!("UCS  option [Face/OBject/World/View/3Point/Z/X/Y/Origin/Save/Delete] or name:")
                    .into_owned()
            }
            Some("Z") => t!("UCS  rotation angle about Z (degrees):").into_owned(),
            Some("X") => t!("UCS  rotation angle about X (degrees):").into_owned(),
            Some("Y") => t!("UCS  rotation angle about Y (degrees):").into_owned(),
            Some("ORIGIN") | Some("O") => t!("UCS  new origin point:").into_owned(),
            Some("3POINT") | Some("3P") => match self.points.len() {
                0 => t!("UCS  specify new origin:").into_owned(),
                1 => t!("UCS  specify point on positive X axis:").into_owned(),
                _ => t!("UCS  specify point in positive XY plane:").into_owned(),
            },
            Some("SAVE") | Some("S") => t!("UCS  name to save current UCS as:").into_owned(),
            Some("DELETE") | Some("DEL") | Some("D") => {
                t!("UCS  name of UCS to delete:").into_owned()
            }
            Some(_) => t!("UCS  value:").into_owned(),
        }
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    /// The new X and Y axes, drawn from the origin while the 3-point UCS is
    /// being placed.
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        if !matches!(self.option.as_deref(), Some("3POINT") | Some("3P")) {
            return None;
        }
        let (origin, x, y) = match self.points.as_slice() {
            [origin] => (*origin, pt - *origin, self.plane.z.cross(pt - *origin)),
            [origin, on_x] => {
                let x = *on_x - *origin;
                let d = pt - *origin;
                (*origin, x, d - x * (d.dot(x) / x.length_squared().max(1.0e-300)))
            }
            _ => return None,
        };
        let y = y.normalize_or_zero() * x.length();
        if y == DVec3::ZERO {
            return None;
        }
        let corner = [origin + x, origin, origin + y].map(|p| p.to_array());
        let (points, points_low) = crate::scene::convert::tessellate::points_to_ds(corner);
        let mut wire = WireModel::solid("ucs_preview".into(), points, WireModel::CYAN, false);
        wire.points_low = points_low;
        Some(wire)
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let t = text.trim();
        if matches!(self.option.as_deref(), None | Some("3POINT") | Some("3P"))
            && crate::app::helpers::parse_coord(t).is_some()
        {
            return None;
        }
        match self.option.take() {
            // First token: the option keyword (or a named UCS to restore).
            None => {
                if t.is_empty() {
                    // Bare Enter → list (delegate to inline `UCS`).
                    return Some(CmdResult::Dispatch("UCS LIST".into()));
                }
                let up = t.to_uppercase();
                if Self::is_zero_arg(&up) {
                    Some(CmdResult::Dispatch(format!("UCS {up}")))
                } else if Self::takes_value(&up) || Self::takes_point(&up) {
                    // Needs a value next; keep the command active and re-prompt.
                    self.option = Some(up);
                    Some(CmdResult::NeedPoint)
                } else {
                    // Not a keyword → a named UCS to activate.
                    Some(CmdResult::Dispatch(format!("UCS {t}")))
                }
            }
            // Second token: the option's value → run the assembled command.
            Some(opt) => Some(CmdResult::Dispatch(format!("UCS {opt} {t}"))),
        }
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        // A click at the option prompt is the new origin; the X axis and XY
        // plane points follow, each optional.
        if self.option.is_none() {
            self.option = Some("3P".into());
        }
        if matches!(self.option.as_deref(), Some("3POINT") | Some("3P")) {
            self.points.push(pt);
            if self.points.len() < 3 {
                return CmdResult::NeedPoint;
            }
            let points = std::mem::take(&mut self.points);
            self.option = None;
            return CmdResult::Dispatch(format!(
                "UCS 3POINTW {},{},{}|{},{},{}|{},{},{}",
                points[0].x,
                points[0].y,
                points[0].z,
                points[1].x,
                points[1].y,
                points[1].z,
                points[2].x,
                points[2].y,
                points[2].z,
            ));
        }
        // A clicked point only makes sense for the Origin option; otherwise a
        // stray click is ignored (keep waiting for the typed keyword / value).
        if matches!(self.option.as_deref(), Some(o) if Self::takes_point(o)) {
            self.option.take();
            return CmdResult::Dispatch(format!("UCS ORIGINW {},{},{}", pt.x, pt.y, pt.z));
        }
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        let points = std::mem::take(&mut self.points);
        match self.option.take() {
            // No option chosen yet → behave like bare `UCS` (list).
            None => CmdResult::Dispatch("UCS LIST".into()),
            // Enter accepts the axes placed so far: the origin alone moves the
            // UCS, an X point turns it about its current Z.
            Some(opt) if matches!(opt.as_str(), "3POINT" | "3P") && !points.is_empty() => {
                let origin = points[0];
                let Some(&on_x) = points.get(1) else {
                    return CmdResult::Dispatch(format!(
                        "UCS ORIGINW {},{},{}",
                        origin.x, origin.y, origin.z
                    ));
                };
                let y = self.plane.z.cross(on_x - origin);
                let on_xy = origin + if y == DVec3::ZERO { self.plane.y } else { y };
                CmdResult::Dispatch(format!(
                    "UCS 3POINTW {},{},{}|{},{},{}|{},{},{}",
                    origin.x, origin.y, origin.z, on_x.x, on_x.y, on_x.z, on_xy.x, on_xy.y, on_xy.z,
                ))
            }
            // Option chosen but no value supplied → cancel cleanly.
            Some(_) => CmdResult::Cancel,
        }
    }
}
