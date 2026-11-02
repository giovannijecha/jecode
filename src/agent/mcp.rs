use super::Agent;
use std::path::Path;

impl Agent {
    pub fn configure_mcp(&mut self, home: &Path) -> Result<(), String> {
        crate::mcp::include_credentials(home, &self.redactor)?;
        self.tools.configure_mcp(home, self.redactor.clone());
        Ok(())
    }
}
