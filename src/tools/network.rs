use super::Tools;
use crate::{cancel::Cancellation, json::Value};

impl Tools {
    pub(super) fn browser(
        &self,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        let mut result = self.browsers.execute(
            self.temporary()?,
            self.browser_profiles.as_deref(),
            arguments,
            cancellation,
        )?;
        if let Some(path) = result
            .get("screenshot_path")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            let arguments = Value::object([("path", Value::string(path))]);
            let (root, arguments) = self.file_arguments(&arguments, false)?;
            let attachment = super::attachment::read_file(
                self.attachments.as_ref(),
                &root,
                &arguments,
                cancellation,
            )?;
            if let (Value::Object(result), Value::Object(attachment)) = (&mut result, attachment) {
                result.extend(attachment);
            }
        }
        Ok(result)
    }
}
