use super::Agent;
use std::path::Path;

impl Agent {
    pub fn configure_lsp(&mut self, home: &Path) -> Result<(), String> {
        self.tools.configure_lsp(home, self.redactor.clone())
    }

    pub fn lsp_runtime(&self) -> crate::lsp::Runtime {
        self.tools.lsp_runtime()
    }
}
