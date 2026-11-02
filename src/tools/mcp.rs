use super::Tools;
use crate::{json::Value, redact::Redactor};
use std::path::Path;

impl Tools {
    /// MCP connections are global, so collaborators use the user's home
    /// rather than their private session home.
    pub fn configure_mcp(&mut self, home: &Path, redactor: Redactor) {
        self.mcp_home = Some(home.to_path_buf());
        self.mcp_redactor = redactor;
    }

    pub fn mcp_label(&self, arguments: &Value) -> Option<String> {
        crate::mcp::protected_label(self.mcp_home.as_deref()?, arguments, &self.mcp_redactor)
    }
}
