use super::Tools;
use crate::{json::Value, redact::Redactor};
use std::path::Path;

impl Tools {
    pub fn configure_mcp(&mut self, home: &Path, redactor: Redactor) {
        if self.session_home.as_deref() != Some(home) {
            self.mcp_runtime = crate::mcp::Runtime::default();
        }
        self.session_home = Some(home.to_path_buf());
        self.mcp_redactor = redactor;
    }

    pub fn mcp_label(&self, arguments: &Value) -> Option<String> {
        if let Some(home) = &self.session_home {
            let _ = crate::mcp::include_credentials(home, &self.mcp_redactor);
        }
        crate::mcp::label(self.session_home.as_deref(), arguments)
    }
}
