// DIMOVERRIDE — set or clear per-object dimension-variable overrides.
//
// Variables and their values are gathered here; the assembled list is handed
// to `DIMOVERRIDE_APPLY <NAME=value;…>` (or `DIMOVERRIDE_APPLY CLEAR`), which
// asks for the dimensions and writes the overrides.

use glam::DVec3;

use crate::command::{CadCommand, CmdOption, CmdResult, InputKind};
use crate::entities::dim_override::{named_var, parse_var_value, VarKind};
use crate::t;

pub struct DimOverrideCommand {
    vars: Vec<(String, String)>,
    pending: Option<(String, VarKind)>,
}

impl DimOverrideCommand {
    pub fn new() -> Self {
        Self {
            vars: Vec::new(),
            pending: None,
        }
    }
}

impl CadCommand for DimOverrideCommand {
    fn name(&self) -> &'static str {
        "DIMOVERRIDE"
    }

    fn prompt(&self) -> String {
        match &self.pending {
            None => {
                t!("DIMOVERRIDE  Enter dimension variable name to override or [Clear overrides]:")
                    .into_owned()
            }
            Some((name, _)) => crate::tf!("DIMOVERRIDE  Enter new value for {name}:").into_owned(),
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        if self.pending.is_some() || !self.vars.is_empty() {
            return Vec::new();
        }
        vec![CmdOption::new("Clear overrides", "CLEAR")]
    }

    fn input_kind(&self) -> InputKind {
        if matches!(self.pending, Some((_, VarKind::Text))) {
            InputKind::FreeText
        } else {
            InputKind::SingleToken
        }
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let value = text.trim();
        if value.is_empty() {
            return None;
        }
        let Some((name, kind)) = self.pending.take() else {
            let upper = value.to_ascii_uppercase();
            if self.vars.is_empty() && matches!(upper.as_str(), "C" | "CLEAR") {
                return Some(CmdResult::Dispatch("DIMOVERRIDE_APPLY CLEAR".to_string()));
            }
            return Some(match named_var(&upper) {
                Some((_, kind)) => {
                    self.pending = Some((upper, kind));
                    CmdResult::NeedPoint
                }
                None => CmdResult::ReportError(
                    crate::tf!("DIMOVERRIDE: unknown dimension variable \"{upper}\".").into_owned(),
                ),
            });
        };
        if parse_var_value(kind, value).is_none() {
            self.pending = Some((name, kind));
            return Some(CmdResult::ReportError(
                t!("DIMOVERRIDE: invalid value for this variable.").into_owned(),
            ));
        }
        self.vars.retain(|(known, _)| *known != name);
        self.vars.push((name, value.to_string()));
        Some(CmdResult::NeedPoint)
    }

    fn on_point(&mut self, _point: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.pending.is_some() || self.vars.is_empty() {
            return CmdResult::Cancel;
        }
        let spec: Vec<String> = self
            .vars
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        CmdResult::Dispatch(format!("DIMOVERRIDE_APPLY {}", spec.join(";")))
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

/// Parse a `NAME=value;…` list into record pairs; the error names the first bad entry.
pub fn parse_spec(spec: &str) -> Result<Vec<(i16, codec::xdata::XDataValue)>, String> {
    spec.split(';')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let (name, value) = entry.split_once('=').unwrap_or((entry, ""));
            named_var(name)
                .and_then(|(code, kind)| Some((code, parse_var_value(kind, value)?)))
                .ok_or_else(|| entry.trim().to_string())
        })
        .collect()
}

inventory::submit!(crate::command::CommandRegistration { names: &["DIMOVERRIDE"] });
