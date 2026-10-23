use super::{Api, BASE_URL, Failure, api_error, headers::Headers};
use crate::json::Value;
use crate::{json, process};
use std::process::Command;

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;

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
        // The body travels on stdin; curl imports the key from its own child
        // environment. Neither value appears in arguments or request files.
        let url = format!("{}{path}", self.base_url);
        let mut command = Command::new(curl_executable());
        command.args([
            "--disable",
            "--silent",
            "--show-error",
            "--no-buffer",
            "--variable",
            "%OPENROUTER_API_KEY",
            "--expand-header",
            "Authorization: Bearer {{OPENROUTER_API_KEY}}",
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
        ]);
        crate::auth::isolate_environment(&mut command);
        command.env("OPENROUTER_API_KEY", &self.api_key);
        if body.is_some() {
            command.args([
                "--header",
                "Content-Type: application/json",
                "--header",
                "Accept: */*",
                "--header",
                "Expect:",
                "--data-binary",
                "@-",
            ]);
        }
        if path == "/chat/completions" {
            command.args(["--header", "X-OpenRouter-Metadata: enabled"]);
        }
        if self.base_url == BASE_URL {
            command.args(["--proto", "=https"]);
        } else {
            // Only the test constructor can select a loopback HTTP fixture.
            command.args(["--noproxy", "*"]);
        }
        let mut headers = Headers::default();
        let output = process::run_observed(
            &mut command,
            body.map(|body| body.as_bytes().to_vec()),
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
        self.response(output, headers.retry_after)
    }

    fn response(
        &self,
        output: process::Output,
        retry_after: Option<std::time::Duration>,
    ) -> Result<String, Failure> {
        if output.cancelled {
            return Err("Operation cancelled".into());
        }
        if output.timed_out {
            return Err(Failure::temporary("OpenRouter connection became inactive"));
        }
        let input_error = output.input_error.as_deref();
        if output.exit_code != Some(0) {
            let message = format!(
                "Could not reach OpenRouter via curl (exit {}): {}",
                output
                    .exit_code
                    .map_or_else(|| "unknown".into(), |code| code.to_string()),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return Err(with_input_error(
                if matches!(
                    output.exit_code,
                    Some(5 | 6 | 7 | 18 | 28 | 35 | 52 | 55 | 56 | 92)
                ) {
                    Failure::temporary(message)
                } else {
                    Failure::from(message)
                },
                input_error,
            ));
        }
        let response = String::from_utf8(output.stdout).map_err(|_| {
            with_input_error("OpenRouter returned invalid UTF-8".into(), input_error)
        })?;
        let (body, status) = response
            .rsplit_once('\n')
            .ok_or_else(|| with_input_error("curl returned no HTTP status".into(), input_error))?;
        let body = super::headers::body(body)
            .map_err(|error| with_input_error(error.into(), input_error))?;
        let status: u16 = status.trim().parse().map_err(|_| {
            with_input_error("curl returned an invalid HTTP status".into(), input_error)
        })?;
        let parsed = json::parse(body);
        if !(200..300).contains(&status) {
            let mut failure = self.failure(parsed.as_ref().unwrap_or(&Value::Null), status);
            failure.retry_after = retry_after;
            return Err(with_input_error(failure, input_error));
        }
        if let Some(error) = input_error {
            return Err(Failure::from(format!(
                "Could not send OpenRouter request to curl: {error}"
            )));
        }
        Ok(body.into())
    }
}

fn with_input_error(mut failure: Failure, error: Option<&str>) -> Failure {
    if let Some(error) = error {
        failure.message.push_str(&format!("; {error}"));
    }
    failure
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
