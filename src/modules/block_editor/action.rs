//! Reuse the command host's point picking and object-set selection.
use crate::command::{CadCommand, CmdOption, CmdResult, PointPickContext};
use crate::scene::block_actions::{self as actions, Kind, Parameter, XyKeyPoint};
use codec::Handle;
use glam::DVec3;

inventory::submit!(crate::command::CommandRegistration {
    names: &["BACTION", "BACTIONTOOL", "BARRAY"]
});

pub(crate) struct ActionCommand {
    document: u64,
    definition: Handle,
    kind: Option<Kind>,
    candidates: Vec<Parameter>,
    parameter: Option<Handle>,
    xy_key_point: Option<XyKeyPoint>,
    targets: Vec<Handle>,
    selecting: bool,
    frame: Vec<DVec3>,
    column_offset: Option<f64>,
    row_offset: Option<f64>,
    context: Option<PointPickContext>,
}
impl ActionCommand {
    pub(crate) fn new(
        document: u64,
        definition: Handle,
        kind: Option<Kind>,
        candidates: Vec<Parameter>,
    ) -> Self {
        Self {
            document,
            definition,
            kind,
            candidates,
            parameter: None,
            xy_key_point: None,
            targets: vec![],
            selecting: false,
            frame: vec![],
            column_offset: None,
            row_offset: None,
            context: None,
        }
    }
    pub(crate) fn for_parameter(
        document: u64,
        definition: Handle,
        kind: Kind,
        candidates: Vec<Parameter>,
        parameter: Handle,
        xy_key_point: Option<XyKeyPoint>,
    ) -> Option<Self> {
        let mut command = Self::new(document, definition, Some(kind), candidates);
        if !command.candidates.iter().any(|candidate| {
            candidate.handle == parameter && actions::compatible(candidate, kind)
        }) {
            return None;
        }
        command.parameter = Some(parameter);
        command.xy_key_point = xy_key_point;
        command.selecting = !actions::needs_frame(kind);
        Some(command)
    }
    fn choose(&mut self, p: Parameter) -> CmdResult {
        if self.kind.is_some_and(|kind| !actions::compatible(&p, kind)) {
            return CmdResult::ReportError(
                crate::t!("Select a compatible parameter for this action.").into_owned(),
            );
        }
        self.parameter = Some(p.handle);
        self.selecting = !self.kind.is_some_and(actions::needs_frame);
        CmdResult::DeselectAndContinue
    }
}
impl CadCommand for ActionCommand {
    fn name(&self) -> &'static str {
        "BACTION"
    }
    fn input_kind(&self) -> crate::command::InputKind {
        if self.parameter.is_some() {
            if self.kind == Some(Kind::Array) && !self.selecting {
                let is_xy = self
                    .candidates
                    .iter()
                    .find(|p| Some(p.handle) == self.parameter)
                    .is_some_and(|p| p.xy);
                if self.column_offset.is_none() || (is_xy && self.row_offset.is_none()) {
                    return crate::command::InputKind::SingleToken;
                }
            }
            crate::command::InputKind::Point
        } else if self.kind.is_some() {
            crate::command::InputKind::FreeText
        } else {
            crate::command::InputKind::SingleToken
        }
    }
    fn prompt(&self) -> String {
        if self.kind.is_none() {
            return crate::t!(
                "BACTION  Enter action type [Move/Scale/Stretch/Polar Stretch/Rotate/Flip/Array]:"
            )
            .into_owned();
        }
        if self.parameter.is_none() {
            return crate::tf!(
                "BACTION {}  Select a parameter endpoint or enter its name:",
                crate::t!(self.kind.unwrap().name())
            )
            .into_owned();
        }
        if self.kind.is_some_and(actions::needs_frame) && self.frame.is_empty() {
            return crate::t!("BACTION  Specify first corner of action frame:").into_owned();
        }
        if self.kind.is_some_and(actions::needs_frame) && self.frame.len() == 1 {
            return crate::t!("BACTION  Specify opposite corner of action frame:").into_owned();
        }
        if self.selecting {
            if let Some(point) = self.xy_key_point {
                let point = match point {
                    XyKeyPoint::Base => "base",
                    XyKeyPoint::X => "X-axis",
                    XyKeyPoint::Y => "Y-axis",
                    XyKeyPoint::End => "end",
                };
                return crate::tf!("BACTION XY Move Box Set  Select objects for the {point} key point, then press Enter:").into_owned();
            }
            return crate::t!("BACTION  Select objects for the action, then press Enter:")
                .into_owned();
        }
        if self.kind == Some(Kind::Array) {
            let is_xy = self
                .candidates
                .iter()
                .find(|p| Some(p.handle) == self.parameter)
                .is_some_and(|p| p.xy);
            if self.column_offset.is_none() {
                return crate::t!(
                    "BACTION  Specify distance between columns (or distance between items):"
                )
                .into_owned();
            }
            if is_xy && self.row_offset.is_none() {
                return crate::t!("BACTION  Specify distance between rows:").into_owned();
            }
        }
        crate::t!("BACTION  Specify action marker location:").into_owned()
    }
    fn options(&self) -> Vec<CmdOption> {
        if self.kind.is_none() {
            vec![
                CmdOption::new("Move", "Move"),
                CmdOption::new("Scale", "Scale"),
                CmdOption::new("Stretch", "Stretch"),
                CmdOption::new("Polar Stretch", "Polar Stretch"),
                CmdOption::new("Rotate", "Rotate"),
                CmdOption::new("Flip", "Flip"),
                CmdOption::new("Array", "Array"),
            ]
        } else {
            vec![]
        }
    }
    fn on_text_input(&mut self, s: &str) -> Option<CmdResult> {
        if self.kind.is_none() {
            return Some(if let Some(k) = Kind::parse(s) {
                self.kind = Some(k);
                CmdResult::NeedPoint
            } else {
                CmdResult::ReportError(
                    crate::t!("Choose Move, Scale, Stretch, Polar Stretch, Rotate, Flip or Array.")
                        .into_owned(),
                )
            });
        }
        if self.parameter.is_none() {
            let matches = self
                .candidates
                .iter()
                .filter(|p| {
                    p.name.eq_ignore_ascii_case(s) && actions::compatible(p, self.kind.unwrap())
                })
                .cloned()
                .collect::<Vec<_>>();
            return Some(if matches.len() == 1 {
                self.choose(matches[0].clone())
            } else {
                CmdResult::ReportError(
                    crate::t!("Enter a unique compatible parameter name or pick its endpoint.")
                        .into_owned(),
                )
            });
        }
        if self.kind == Some(Kind::Array) && !self.selecting {
            let is_xy = self
                .candidates
                .iter()
                .find(|p| Some(p.handle) == self.parameter)
                .is_some_and(|p| p.xy);
            if self.column_offset.is_none() {
                let Ok(dist) = s.trim().parse::<f64>() else {
                    return Some(CmdResult::ReportError(
                        crate::t!("Specify a positive distance.").into_owned(),
                    ));
                };
                if dist <= 1e-6 || !dist.is_finite() {
                    return Some(CmdResult::ReportError(
                        crate::t!("Specify a positive distance.").into_owned(),
                    ));
                }
                self.column_offset = Some(dist);
                return Some(CmdResult::NeedPoint);
            }
            if is_xy && self.row_offset.is_none() {
                let Ok(dist) = s.trim().parse::<f64>() else {
                    return Some(CmdResult::ReportError(
                        crate::t!("Specify a positive distance.").into_owned(),
                    ));
                };
                if dist <= 1e-6 || !dist.is_finite() {
                    return Some(CmdResult::ReportError(
                        crate::t!("Specify a positive distance.").into_owned(),
                    ));
                }
                self.row_offset = Some(dist);
                return Some(CmdResult::NeedPoint);
            }
        }
        None
    }
    fn wants_point_pick_context(&self) -> bool {
        self.kind.is_some() && self.parameter.is_none()
    }
    fn set_point_pick_context(&mut self, c: Option<PointPickContext>) {
        self.context = c;
    }
    fn on_point(&mut self, point: DVec3) -> CmdResult {
        if self.parameter.is_none() {
            let Some(c) = self.context else {
                return CmdResult::ReportError(crate::t!("Enter the parameter name.").into_owned());
            };
            let screen =
                |p| crate::scene::pick::hit_test::world_to_screen(p, c.view, c.eye, c.bounds);
            let click = screen(point);
            let candidate = self
                .candidates
                .iter()
                .filter(|p| actions::compatible(p, self.kind.unwrap()))
                .map(|p| (screen(p.end).distance(click), p))
                .filter(|(d, _)| d.is_finite() && *d <= c.aperture_px.max(10.))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p)| p.clone());
            return candidate.map(|p| self.choose(p)).unwrap_or_else(|| {
                CmdResult::ReportError(
                    crate::t!("Pick a compatible parameter endpoint.").into_owned(),
                )
            });
        }
        if self.kind.is_some_and(actions::needs_frame) && self.frame.len() < 2 {
            let plane = self
                .candidates
                .iter()
                .find(|p| Some(p.handle) == self.parameter)
                .unwrap()
                .end
                .z;
            if !point.is_finite() || (point.z - plane).abs() > 1e-8 {
                return CmdResult::ReportError(
                    crate::t!("Specify a finite corner in the parameter's XY plane.").into_owned(),
                );
            }
            if self
                .frame
                .first()
                .is_some_and(|a| (point.x - a.x).abs() < 1e-9 || (point.y - a.y).abs() < 1e-9)
            {
                return CmdResult::ReportError(
                    crate::t!("The stretch frame needs a nonzero width and height.").into_owned(),
                );
            }
            self.frame.push(point);
            if self.frame.len() == 2 {
                self.selecting = true;
                return CmdResult::DeselectAndContinue;
            }
            return CmdResult::NeedPoint;
        }
        if self.kind == Some(Kind::Array) && !self.selecting {
            let p = self
                .candidates
                .iter()
                .find(|p| Some(p.handle) == self.parameter)
                .unwrap();
            if self.column_offset.is_none() {
                let dist = point.distance(p.base);
                if dist <= 1e-6 || !dist.is_finite() {
                    return CmdResult::ReportError(
                        crate::t!("Specify a positive distance.").into_owned(),
                    );
                }
                self.column_offset = Some(dist);
                return CmdResult::NeedPoint;
            }
            if p.xy && self.row_offset.is_none() {
                let dist = (point.y - p.base.y).abs().max(point.distance(p.base));
                if dist <= 1e-6 || !dist.is_finite() {
                    return CmdResult::ReportError(
                        crate::t!("Specify a positive distance.").into_owned(),
                    );
                }
                self.row_offset = Some(dist);
                return CmdResult::NeedPoint;
            }
        }
        if self.selecting {
            return CmdResult::NeedPoint;
        }
        if !point.is_finite() {
            return CmdResult::ReportError(
                crate::t!("Specify a finite action location.").into_owned(),
            );
        }
        let array_offsets = if self.kind == Some(Kind::Array) {
            let col = self.column_offset.unwrap_or(1.0);
            let row = self.row_offset.unwrap_or(1.0);
            Some((col, row))
        } else {
            None
        };
        CmdResult::PlaceBlockAction {
            document: self.document,
            definition: self.definition,
            kind: self.kind.unwrap(),
            parameter: self.parameter.unwrap(),
            xy_key_point: self.xy_key_point,
            targets: self.targets.clone(),
            location: point,
            frame: (self.frame.len() == 2).then(|| [self.frame[0], self.frame[1]]),
            array_offsets,
        }
    }

    fn on_preview_wires(&mut self, point: DVec3) -> Vec<crate::scene::WireModel> {
        frame_preview(&self.frame, point)
    }

    fn is_selection_gathering(&self) -> bool {
        self.selecting
    }
    fn on_selection_complete(&mut self, handles: Vec<Handle>) -> CmdResult {
        // The host calls this after every pick/window/removal, not just Enter.
        // Retain the current set while the user gathers additional objects.
        self.targets = handles;
        CmdResult::NeedPoint
    }
    fn on_enter(&mut self) -> CmdResult {
        if self.selecting {
            if self.targets.is_empty() {
                return CmdResult::ReportError(
                    crate::t!("Select at least one object for the action.").into_owned(),
                );
            }
            self.selecting = false;
            return CmdResult::NeedPoint;
        }
        CmdResult::Cancel
    }
}

pub(crate) struct TestParameterCommand {
    pub insert: Option<Handle>,
    pub document: u64,
    pub definition: Handle,
    pub parameter: Handle,
}
impl CadCommand for TestParameterCommand {
    fn name(&self) -> &'static str {
        if self.insert.is_some() {
            "BLOCKPARAMETERGRIP"
        } else {
            "BTESTBLOCK"
        }
    }
    fn prompt(&self) -> String {
        crate::t!("Specify the new block parameter endpoint (Escape cancels):").into_owned()
    }
    fn on_point(&mut self, position: DVec3) -> CmdResult {
        CmdResult::EvaluateBlockParameter {
            insert: self.insert,
            document: self.document,
            definition: self.definition,
            parameter: self.parameter,
            value: actions::Value::Point(position),
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

/// Shared rectangular frame preview for authoring and reauthoring.
pub(super) fn frame_preview(frame: &[DVec3], point: DVec3) -> Vec<crate::scene::WireModel> {
    let Some(a) = frame.first().filter(|_| point.is_finite()) else {
        return vec![];
    };
    let b = frame.get(1).copied().unwrap_or(point);
    vec![crate::scene::WireModel::solid_f64(
        "preview_stretch_frame".into(),
        vec![
            a.to_array(),
            [b.x, a.y, a.z],
            [b.x, b.y, a.z],
            [a.x, b.y, a.z],
        ],
        crate::scene::WireModel::CYAN,
        true,
    )]
}
