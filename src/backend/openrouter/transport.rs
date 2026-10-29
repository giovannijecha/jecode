use super::{Api, Failure, api_error, headers::Headers};
use crate::json::Value;
use crate::{json, process};

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
        // The request, including its token, travels on private child stdin.
        let url = format!("{}{path}", self.base_url);
        let authorization = format!("Bearer {}", self.api_key);
        let mut request_headers = vec![("Authorization", authorization.as_str())];
        if body.is_some() {
            request_headers.extend([("Content-Type", "application/json"), ("Accept", "*/*")]);
        }
        if path == "/chat/completions" {
            request_headers.push(("X-OpenRouter-Metadata", "enabled"));
        }
        let mut headers = Headers::default();
        let mut output = crate::http::run(
            crate::http::Request {
                url: &url,
                method,
                headers: &request_headers,
                body: body.map(str::as_bytes),
                timeout: None,
                idle: Some(std::time::Duration::from_secs(120)),
                output_limit: usize::MAX,
            },
            &self.cancellation,
            &mut |bytes| headers.feed(bytes, observer),
        )
        .map_err(|error| {
            let mut error = Failure::from(error);
            error.retry_after = headers.retry_after;
            error
        })?;
        // Preserve the response decoder's bounded HTTP/error behavior.
        output
            .stdout
            .extend_from_slice(format!("\n{}", headers.status).as_bytes());
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
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = format!(
                "Could not reach OpenRouter via native HTTP (exit {}): {}",
                output
                    .exit_code
                    .map_or_else(|| "unknown".into(), |code| code.to_string()),
                stderr.trim(),
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
            .ok_or_else(|| with_input_error("HTTP response had no status".into(), input_error))?;
        let body = super::headers::body(body)
            .map_err(|error| with_input_error(error.into(), input_error))?;
        let status: u16 = status.trim().parse().map_err(|_| {
            with_input_error("HTTP response had an invalid status".into(), input_error)
        })?;
        let parsed = json::parse(body);
        if !(200..300).contains(&status) {
            let mut failure = self.failure(parsed.as_ref().unwrap_or(&Value::Null), status);
            failure.retry_after = retry_after;
            return Err(with_input_error(failure, input_error));
        }
        if let Some(error) = input_error {
            return Err(Failure::from(format!(
                "Could not send OpenRouter HTTP request: {error}"
            )));
        }
        Ok(body.into())
    }
}

fn with_input_error(mut failure: Failure, error: Option<&str>) -> Failure {
    if let Some(error) = error {
        let detail = format!("; {error}");
        failure.message.push_str(&detail);
        if let Some(diagnostic) = &mut failure.diagnostic {
            diagnostic.push_str(&detail);
        }
    }
    failure
}
