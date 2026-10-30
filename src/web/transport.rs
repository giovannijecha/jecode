use super::url::Url;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub(super) struct Options {
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub timeout_seconds: usize,
    pub max_bytes: Option<usize>,
}

impl Options {
    pub fn from(arguments: &Value) -> Result<Self, String> {
        let method = arguments.get("method").and_then(Value::as_str).unwrap_or(
            if arguments.get("body").is_some() {
                "POST"
            } else {
                "GET"
            },
        );
        if !is_token(method) {
            return Err("method must be an HTTP token".into());
        }
        let mut headers = Vec::new();
        if let Some(value) = arguments.get("headers") {
            let Value::Object(entries) = value else {
                return Err("headers must be an object of strings".into());
            };
            for (name, value) in entries {
                let Some(value) = value.as_str() else {
                    return Err("headers must be an object of strings".into());
                };
                if !is_token(name) || value.contains(['\r', '\n']) {
                    return Err("Invalid HTTP header".into());
                }
                headers.push((name.clone(), value.to_string()));
            }
        }
        let body = arguments
            .get("body")
            .map(|value| value.as_str().ok_or("body must be a string"))
            .transpose()?
            .map(str::to_string);
        let timeout_seconds = positive(arguments, "timeout_seconds", 30)?;
        let max_bytes = arguments
            .get("max_bytes")
            .map(|_| positive(arguments, "max_bytes", 0))
            .transpose()?;
        Ok(Self {
            method: method.to_string(),
            headers,
            body,
            timeout_seconds,
            max_bytes,
        })
    }
}

fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn positive(arguments: &Value, key: &str, default: usize) -> Result<usize, String> {
    match arguments.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_usize()
            .filter(|number| *number > 0)
            .ok_or_else(|| format!("{key} must be a positive integer")),
    }
}

pub(super) struct Response {
    pub final_url: Url,
    pub status: u16,
    pub mime: Option<String>,
    pub bytes: u64,
    pub fetched_at: u64,
    pub alias: String,
    pub path: PathBuf,
}

pub(super) fn fetch(
    area: &Area,
    url: &Url,
    options: &Options,
    header_origins: &[Url],
    prefix: &str,
    cancellation: &Cancellation,
) -> Result<Response, String> {
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    let alias = format!("{prefix}/{}.body", unique_id());
    let (_, path) = area.file(&alias, true)?;
    let result = run(&path, url, options, header_origins, cancellation);
    match result {
        Ok((status, final_url, mime)) => {
            let bytes = fs::metadata(&path)
                .map_err(|error| format!("Could not inspect downloaded file: {error}"))?
                .len();
            if options.max_bytes.is_some_and(|limit| bytes > limit as u64) {
                let _ = fs::remove_file(&path);
                return Err("Response exceeds max_bytes".into());
            }
            Ok(Response {
                final_url,
                status,
                mime,
                bytes,
                fetched_at: now(),
                alias,
                path,
            })
        }
        Err(error) => {
            let _ = fs::remove_file(&path);
            Err(error)
        }
    }
}

fn run(
    path: &PathBuf,
    url: &Url,
    options: &Options,
    header_origins: &[Url],
    cancellation: &Cancellation,
) -> Result<(u16, Url, Option<String>), String> {
    let started = Instant::now();
    let timeout = Duration::from_secs(options.timeout_seconds as u64);
    let mut current = url.clone();
    let mut options = options.clone();
    for redirects in 0..=50 {
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or("HTTP transfer failed: timed out")?;
        let send_headers = header_origins
            .iter()
            .any(|origin| origin.same_origin(&current));
        let (status, final_url, mime, redirect) = run_once(
            path,
            &current,
            &options,
            send_headers,
            remaining,
            cancellation,
        )?;
        let Some(next) = redirect.filter(|_| matches!(status, 301 | 302 | 303 | 307 | 308)) else {
            return Ok((status, final_url, mime));
        };
        if redirects == 50 {
            return Err("HTTP transfer exceeded 50 redirects".into());
        }
        if (options.method == "POST" && matches!(status, 301 | 302))
            || (status == 303 && options.method != "HEAD")
        {
            options.method = "GET".into();
            options.body = None;
            options.headers.retain(|(name, _)| {
                ![
                    "Content-Length",
                    "Transfer-Encoding",
                    "Content-Type",
                    "Content-Encoding",
                    "Expect",
                ]
                .iter()
                .any(|header| name.eq_ignore_ascii_case(header))
            });
        }
        current = Url::parse(&next)?;
    }
    unreachable!()
}

type Hop = (u16, Url, Option<String>, Option<String>);

fn run_once(
    path: &PathBuf,
    url: &Url,
    options: &Options,
    send_headers: bool,
    timeout: Duration,
    cancellation: &Cancellation,
) -> Result<Hop, String> {
    let mut file =
        fs::File::create(path).map_err(|error| format!("Could not open download: {error}"))?;
    let mut decoder = crate::http::Decoder::default();
    let mut size = 0usize;
    let headers = if send_headers {
        options
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let output = crate::http::run(
        crate::http::Request {
            url: &url.value,
            method: &options.method,
            headers: &headers,
            body: options.body.as_ref().map(|body| body.as_bytes()),
            timeout: Some(timeout),
            idle: None,
            output_limit: 0,
        },
        cancellation,
        &mut |bytes| {
            decoder.feed(bytes, &mut |part| {
                size = size.saturating_add(part.len());
                if options.max_bytes.is_some_and(|limit| size > limit) {
                    return Err("Response exceeds max_bytes".into());
                }
                file.write_all(part)
                    .map_err(|error| format!("Could not write download: {error}"))
            })
        },
    )?;
    if output.cancelled {
        return Err("Operation cancelled".into());
    }
    if output.timed_out {
        return Err("HTTP transfer failed: timed out".into());
    }
    if output.exit_code != Some(0) {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("HTTP transfer failed: {}", detail.trim()));
    }
    if let Some(error) = output.input_error {
        return Err(error);
    }
    let status = decoder.status.ok_or("Could not read HTTP status")?;
    let mime = decoder.header("content-type").map(str::to_string);
    let redirect = decoder
        .header("location")
        .map(|value| {
            url.resolve(value)
                .map(|target| target.value)
                .ok_or("Invalid HTTP redirect location".to_string())
        })
        .transpose()?;
    Ok((status, url.clone(), mime, redirect))
}

pub(super) fn unique_id() -> String {
    format!(
        "{}-{}-{}",
        now(),
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
