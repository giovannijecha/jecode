//! Session-scoped stores, areas and homes that configure the tools.

use super::{Tools, processes};
use std::path::Path;

impl Tools {
    #[cfg(test)]
    pub fn configure_output(
        &mut self,
        directory: std::path::PathBuf,
        redactor: crate::redact::Redactor,
    ) {
        self.outputs = crate::output::Store::new(directory, redactor);
    }
    #[cfg(test)]
    pub fn configure_session_output(
        &mut self,
        directory: std::path::PathBuf,
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
    /// The agent's collaborators, whose temporary files it reads.
    pub(crate) fn configure_collaborators(
        &mut self,
        collaborators: crate::collaboration::Observer,
    ) {
        self.collaborators = collaborators;
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
