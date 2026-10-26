//! DIM and the steps it hands to the app (Layer, aliGn, Undo).

use crate::app::{Message, OpenCADStudio};
use crate::command::CadCommand;
use iced::Task;

impl OpenCADStudio {
    fn start_dim(&mut self, i: usize) {
        use crate::modules::annotate::dim_cmd::DimCommand;
        let layer = crate::io::drawing_variable(&self.tabs[i].scene.document, "DIMLAYER")
            .map(|layer| layer.trim().to_string())
            .filter(|layer| !layer.is_empty() && layer != ".")
            .unwrap_or_else(|| crate::tr!("dim-cmd", "use-current"));
        let command = DimCommand::new(layer);
        self.command_line.push_info(&command.prompt());
        self.command_line.set_step_options(command.options());
        self.tabs[i].active_cmd = Some(Box::new(command));
    }

    pub(super) fn dispatch_dim_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (name, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
        match name.trim_start_matches(['_', '.']).to_ascii_uppercase().as_str() {
            "DIM" => self.start_dim(i),
            // Layer: DIMLAYER for the dimensions DIM makes next.
            "DIM_LAYER" => {
                let task = self.dispatch_command(&format!("DIMLAYER {}", rest.trim()));
                self.start_dim(i);
                return Some(task);
            }
            "DIM_ALIGN" => {
                use crate::modules::annotate::dimspace::DimSpaceCommand;
                let command = DimSpaceCommand::aligning();
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }
            // Undo: the last dimension goes, DIM goes on.
            "DIM_UNDO" => {
                let task = self.dispatch_command("U");
                self.start_dim(i);
                return Some(task);
            }
            _ => return None,
        }
        Some(Task::none())
    }
}
