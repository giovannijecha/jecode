//! Session-scoped stores, areas and homes that configure the tools.

use super::{Tools, processes};
use crate::json::Value;
use std::path::{Path, PathBuf};

impl Tools {
    #[cfg(test)]
    pub fn configure_output(&mut self, directory: PathBuf, redactor: crate::redact::Redactor) {
        self.outputs = crate::output::Store::new(directory, redactor);
    }
    #[cfg(test)]
    pub fn configure_session_output(
        &mut self,
        directory: PathBuf,
        session: &str,
        redactor: crate::redact::Redactor,
    ) -> Result<(), String> {
        self.outputs =
            crate::output::Store::for_session(directory, self.root.clone(), session, redactor)?;
        Ok(())
    }
    pub(crate) fn configure_output_store(&mut self, outputs: crate::output::Store) {
        self.outputs = outputs;
    }
    /// Lets a collaborator read the saved outputs and temporary files of the
    /// agents above it, which its main agent hands it by reference.
    pub(crate) fn inherit_references(&mut self, parent: &Tools) {
        self.outputs.inherit(&parent.outputs);
        self.inherited_temporary = parent
            .temporary
            .iter()
            .chain(&parent.inherited_temporary)
            .map(crate::scratch::Area::share)
            .collect();
    }

    /// Read arguments. A tmp: file absent from this agent's area is read from
    /// the nearest agent above it that has one; writes stay in its own area.
    pub(super) fn read_arguments(&self, arguments: &Value) -> Result<(PathBuf, Value), String> {
        let error = match self.file_arguments(arguments, false) {
            Err(error) => error,
            found => return found,
        };
        let Some((root, path)) = arguments
            .get("path")
            .and_then(Value::as_str)
            .and_then(|path| path.strip_prefix("tmp:"))
            .and_then(|alias| {
                self.inherited_temporary
                    .iter()
                    .find_map(|area| area.existing(alias))
            })
        else {
            return Err(error);
        };
        let mut arguments = arguments.clone();
        if let Value::Object(fields) = &mut arguments {
            fields.insert("path".into(), Value::string(path.to_string_lossy()));
        }
        Ok((root, arguments))
    }
    pub fn include_credentials(&mut self, redactor: crate::redact::Redactor) {
        self.project_map.include_credentials(redactor.clone());
        self.lsp_redactor.include(redactor.clone());
        self.lsp_runtime
            .include_credentials(self.lsp_redactor.clone());
        self.outputs.include(redactor);
    }

    pub fn configure_project_map(&mut self, home: &Path) {
        self.project_map.configure(home);
    }
    pub fn share_project_map_notes(&mut self, home: &Path, project_root: &Path) {
        self.project_map.share_notes(home, project_root);
    }

    pub fn configure_attachments(&mut self, pool: crate::attachments::Pool) {
        self.attachments = Some(pool);
    }

    pub fn attachments(&self) -> Option<&crate::attachments::Pool> {
        self.attachments.as_ref()
    }

    /// Stops the persistent processes of the conversation that is ending.
    pub fn stop_processes(&self) {
        self.processes.stop_all();
        self.stop_browsers();
        self.lsp_runtime.shutdown();
    }

    pub fn stop_browsers(&self) {
        self.browsers.stop_all();
    }

    pub fn configure_browser_profiles(&mut self, home: &Path) {
        self.browser_profiles = Some(home.join("browser/profiles"));
    }

    pub fn configure_session_home(&mut self, home: &Path) {
        self.session_home = Some(home.to_path_buf());
    }

    /// Local inspection and stop control for the current conversation's processes.
    pub fn observe_processes(&self, redactor: crate::redact::Redactor) -> processes::Observer {
        self.processes.observer(redactor)
    }

    pub fn configure_temporary(&mut self, area: crate::scratch::Area) {
        self.temporary = Some(area);
    }

    pub fn temporary(&self) -> Result<&crate::scratch::Area, String> {
        self.temporary
            .as_ref()
            .ok_or("Session temporary storage is unavailable".into())
    }

    pub fn temporary_instructions(&self) -> String {
        self.temporary
            .as_ref()
            .map_or(String::new(), |area| area.instructions())
    }
}
