use super::{Api, BASE_URL, api_error, failure::Failure, headers::Headers};
use crate::json::Value;
use crate::{json, process};
use std::process::Command;

impl Api {
    pub(super) fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<Value, String> {
        self.send(method, path, body)
            .map_err(|error| self.redact(&error.message))
    }

    pub(super) fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<Value, Failure> {
        let response = self.raw(method, path, body, &mut |_| Ok(()))?;
        let value = json::parse(&response)?;
        if api_error(&value).is_some() {
            return Err(self.failure(&value, 200));
        }
        Ok(value)
    }

    pub(super) fn raw(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        observer: &mut impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<String, Failure> {
        // Credentials and request bodies are passed through stdin, never arguments or files.
        let mut config = format!(
            "header = {}\n",
            config_string(&format!("Authorization: Bearer {}", self.api_key))
        );
        if let Some(body) = body {
            config.push_str(&format!("header = \"Content-Type: application/json\"\nheader = \"Expect:\"\ndata-binary = {}\n", config_string(body)));
        }
        let url = format!("{}{path}", self.base_url);
        let mut command = Command::new(curl_executable());
        command
            .args([
                "--disable",
                "--silent",
                "--show-error",
                "--no-buffer",
                "--request",
                method,
                "--connect-timeout",
                "10",
                "--suppress-connect-headers",
                "--dump-header",
                "-",
                "--write-out",
                "\n%{http_code}",
                "--url",
                &url,
                "--config",
                "-",
            ])
            .env_remove("OPENROUTER_API_KEY");
        if self.base_url == BASE_URL {
            command.args(["--proto", "=https"]);
        } else {
            // Only the test constructor can select a loopback HTTP fixture.
            command.args(["--noproxy", "*"]);
        }
        let mut headers = Headers::default();
        let output = process::run_observed(
            &mut command,
            Some(config.into_bytes()),
            None,
            usize::MAX,
            &self.cancellation,
            Some(std::time::Duration::from_secs(120)),
            &mut |bytes| headers.feed(bytes, observer),
        )
        .map_err(|error| {
            let mut error = Failure::from(error);
            error.retry_after = headers.retry_after;
            error
        })?;
        if output.cancelled {
            return Err("Operation cancelled".into());
        }
        if output.timed_out {
            return Err(Failure::temporary("OpenRouter connection became inactive"));
        }
        if output.exit_code != Some(0) {
            let message = format!(
                "Could not reach OpenRouter via curl: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return Err(
                if matches!(
                    output.exit_code,
                    Some(5 | 6 | 7 | 18 | 28 | 35 | 52 | 55 | 56 | 92)
                ) {
                    Failure::temporary(message)
                } else {
                    Failure::from(message)
                },
            );
        }
        let response =
            String::from_utf8(output.stdout).map_err(|_| "OpenRouter returned invalid UTF-8")?;
        let (body, status) = response
            .rsplit_once('\n')
            .ok_or("curl returned no HTTP status")?;
        let body = super::headers::body(body)?;
        let status: u16 = status
            .trim()
            .parse()
            .map_err(|_| "curl returned an invalid HTTP status")?;
        let parsed = json::parse(body);
        if !(200..300).contains(&status) {
            let mut failure = self.failure(parsed.as_ref().unwrap_or(&Value::Null), status);
            failure.retry_after = headers.retry_after;
            return Err(failure);
        }
        Ok(body.into())
    }
}

fn curl_executable() -> std::ffi::OsString {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let path = std::path::PathBuf::from(root).join("System32/curl.exe");
        if path.is_file() {
            return path.into_os_string();
        }
    }
    "curl".into()
}

fn config_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
