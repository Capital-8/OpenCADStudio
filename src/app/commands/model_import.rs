//! ACISIN and WMFIN, and the files IMPORT routes here: ACIS text (SAT),
//! STEP and IGES models and Windows metafiles, read into the current drawing
//! as one undo step.

use super::*;
use crate::io::model_import::{self, ModelFormat};

impl OpenCADStudio {
    /// ACISIN and WMFIN; `None` defers to the other families.
    pub(super) fn dispatch_model_import(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (verb, rest) = match cmd.trim().split_once(char::is_whitespace) {
            Some((verb, rest)) => (verb.to_ascii_uppercase(), rest.trim().trim_matches('"')),
            None => (cmd.trim().to_ascii_uppercase(), ""),
        };
        match (verb.as_str(), rest.is_empty()) {
            ("ACISIN", true) if crate::sys::file_dia() => {
                let filters = vec![("ACIS (*.sat)".to_string(), vec!["sat".to_string()])];
                let title = crate::t!("Select ACIS File").into_owned();
                Some(Task::perform(super::dgn_import::pick(title, filters), |path| match path {
                    Some(path) => Message::Command(format!("_ACISINFILE {path}")),
                    None => Message::Noop,
                }))
            }
            ("ACISIN", true) => {
                use crate::command::FreeTextValuePromptCommand;
                let command = FreeTextValuePromptCommand::new("_ACISINFILE", "Enter SAT file name:");
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
                Some(Task::none())
            }
            ("ACISIN" | "_ACISINFILE", false) => {
                if crate::scene::model::pdf_raster::source_bytes(rest).is_none() {
                    let name = std::path::Path::new(rest)
                        .file_name()
                        .map_or_else(|| rest.to_string(), |n| n.to_string_lossy().into_owned());
                    let here = std::env::current_dir()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default();
                    self.command_line.push_error(&format!(
                        "\"{name}\": Can't find file in search path:\n  {here}\\ (current directory)"
                    ));
                    return Some(Task::none());
                }
                Some(self.import_model(i, rest, ModelFormat::Sat))
            }
            ("_ACISINFILE", true) => Some(Task::none()),
            ("WMFIN", true) if crate::sys::file_dia() => {
                let filters = vec![("Metafile (*.wmf)".to_string(), vec!["wmf".to_string()])];
                let title = crate::t!("Import WMF").into_owned();
                Some(Task::perform(super::dgn_import::pick(title, filters), |path| match path {
                    Some(path) => Message::Command(format!("_WMFINFILE {path}")),
                    None => Message::Noop,
                }))
            }
            ("WMFIN", true) => {
                use crate::command::FreeTextValuePromptCommand;
                let command = FreeTextValuePromptCommand::new(
                    "_WMFINFILE",
                    "Enter name of WMF file to import (no extension):",
                );
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
                Some(Task::none())
            }
            ("WMFIN" | "_WMFINFILE", false) => {
                let path = if std::path::Path::new(rest).extension().is_some() {
                    rest.to_string()
                } else {
                    format!("{rest}.wmf")
                };
                Some(self.import_wmf(i, &path, true))
            }
            ("_WMFINFILE", true) => Some(Task::none()),
            _ => None,
        }
    }

    /// Reads a model file into drawing `i`.
    pub(in crate::app) fn import_model(&mut self, i: usize, path: &str, format: ModelFormat) -> Task<Message> {
        let Some(bytes) = crate::scene::model::pdf_raster::source_bytes(path) else {
            self.command_line
                .push_error(&format!("File {path} does not exist."));
            return Task::none();
        };
        // The translated formats report the job as they run.
        let translated = format != ModelFormat::Sat;
        if translated {
            self.command_line
                .push_output("Import job in progress.  Press Esc key to cancel.");
        }
        let drawing_mm = crate::app::properties::insunits_to_mm(
            self.tabs[i].scene.document.header.insertion_units,
        );
        let Some(imported) = model_import::read(format, &bytes, drawing_mm) else {
            let shown = path.replace('/', "\\");
            match format {
                ModelFormat::Sat => {
                    self.command_line.push_error("Modeling Operation Error:");
                    self.command_line.push_error("Error Code Number is 58000");
                    self.command_line
                        .push_error(&format!("Unable to read the ACIS file {shown}."));
                }
                _ => self
                    .command_line
                    .push_error(&format!("Import failed: {shown} has no model the importer reads.")),
            }
            return Task::none();
        };
        self.push_undo_snapshot(i, "IMPORT");
        let scene = &mut self.tabs[i].scene;
        // A block the drawing already has keeps its definition and is
        // inserted as it is.
        for (name, entities) in imported.blocks {
            if scene.document.block_records.get(&name).is_none() {
                let _ = scene.define_block_from_owned_entities(entities, &name, glam::DVec3::ZERO);
            }
        }
        for entity in imported.entities {
            scene.add_entity(entity);
        }
        self.tabs[i].dirty = true;
        if translated {
            self.command_line.push_output("Import Job Complete");
        }
        self.refresh_properties();
        Task::none()
    }

    /// A metafile as a new block `WMF<n>`, fitted to half the current view:
    /// placed at the view's top-left corner (IMPORT), or by the insertion
    /// prompts (WMFIN).
    pub(in crate::app) fn import_wmf(&mut self, i: usize, path: &str, prompt: bool) -> Task<Message> {
        let Some(bytes) = crate::scene::model::pdf_raster::source_bytes(path) else {
            self.command_line
                .push_error(&format!("File {path} does not exist."));
            return Task::none();
        };
        let (centre, size, pixel) = self.tabs[i].scene.view_window();
        let Some(mut picture) =
            crate::io::wmf_import::read(&bytes, [0.5 * size[0], 0.5 * size[1]])
        else {
            self.command_line
                .push_error(&format!("{} is not a valid Windows metafile.", path.replace('/', "\\")));
            return Task::none();
        };
        self.push_undo_snapshot(i, if prompt { "WMFIN" } else { "IMPORT" });
        let scene = &mut self.tabs[i].scene;
        // A text style per face, `WMF-<face><n>` with `n` counting the
        // face's bold and italic variants; the style's font is the
        // installed file of that face, none when it is not installed. A
        // style the drawing already has is used as it is.
        let mut styles: Vec<String> = Vec::new();
        for (k, face) in picture.fonts.iter().enumerate() {
            let n = picture.fonts[..k].iter().filter(|f| f.name == face.name).count();
            let name = format!("WMF-{}{n}", face.name);
            if !scene.document.text_styles.contains(&name) {
                let mut style = codec::tables::TextStyle::new(name.as_str());
                style.handle = scene.document.allocate_handle();
                style.font_file =
                    crate::scene::text::sysfont::face_file(&face.name, face.bold, face.italic).unwrap_or_default();
                let _ = scene.document.text_styles.add(style);
            }
            styles.push(name);
        }
        for (at, font) in &picture.text_fonts {
            if let Some(codec::EntityType::Text(text)) = picture.entities.get_mut(*at) {
                text.style = styles[*font].clone();
            }
        }
        let mut n = 0;
        let name = loop {
            let name = format!("WMF{n}");
            if scene.document.block_records.get(&name).is_none() {
                break name;
            }
            n += 1;
        };
        if let Err(error) =
            scene.define_block_from_owned_entities(picture.entities, &name, glam::DVec3::ZERO)
        {
            self.command_line.push_error(&error);
            return Task::none();
        }
        self.tabs[i].dirty = true;
        if prompt {
            use crate::command::CadCommand;
            use crate::modules::insert::insert_block::InsertBlockCommand;
            let preview = self.tabs[i].scene.block_preview_wires(&name);
            let command = InsertBlockCommand::classic_for_block(name, preview, String::new());
            self.command_line.push_info(&command.prompt());
            self.tabs[i].active_cmd = Some(Box::new(command));
        } else {
            // The view's top-left corner, a pixel or two in.
            let corner = codec::types::Vector3::new(
                centre[0] - 0.5 * size[0] + pixel,
                centre[1] + 0.5 * size[1] - 2.0 * pixel,
                0.0,
            );
            let insert = codec::entities::Insert::new(&name, corner);
            self.tabs[i].scene.add_entity(codec::EntityType::Insert(insert));
            self.refresh_properties();
        }
        Task::none()
    }
}
