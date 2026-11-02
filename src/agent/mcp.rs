use super::Agent;
use std::path::Path;

impl Agent {
    /// The tool stays configured when mcp.json is unreadable, so a call
    /// returns that error to the model instead of hiding the tool.
    pub fn configure_mcp(&mut self, home: &Path) -> Result<(), String> {
        self.tools.configure_mcp(home, self.redactor.clone());
        crate::mcp::include_credentials(home, &self.redactor)
    }
}
