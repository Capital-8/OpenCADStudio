// DATAEXTRACTION command — launches the Data Extraction wizard.

use crate::modules::{IconKind, ModuleEvent, ToolDef};

pub const ICON: IconKind = IconKind::Svg(include_bytes!("../../../assets/icons/data_extract.svg"));

pub fn tool() -> ToolDef {
    ToolDef {
        id: "DATAEXTRACTION",
        label: "Extract\nData",
        icon: ICON,
        event: ModuleEvent::Command("DATAEXTRACTION".to_string()),
    }
}

/// -DATAEXTRACTION: asks for a saved extraction's settings file and runs it.
pub struct TemplateCommand;

impl crate::command::CadCommand for TemplateCommand {
    fn name(&self) -> &'static str {
        "-DATAEXTRACTION"
    }

    fn prompt(&self) -> String {
        crate::t!("-DATAEXTRACTION  Enter the template file path for the extraction:").into_owned()
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn on_text_input(&mut self, text: &str) -> Option<crate::command::CmdResult> {
        use crate::command::CmdResult;
        let path = text.trim().trim_matches('"');
        if path.is_empty() {
            return Some(CmdResult::NeedPoint);
        }
        if !std::path::Path::new(path).is_file() {
            return Some(CmdResult::ReportMeasurement(
                crate::t!("Invalid name.").into_owned(),
            ));
        }
        Some(CmdResult::Dispatch(format!("-DATAEXTRACTION _RUN {path}")))
    }

    fn on_point(&mut self, _point: glam::DVec3) -> crate::command::CmdResult {
        crate::command::CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::NeedPoint
    }
}
