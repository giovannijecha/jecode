use super::{
    Browser, logs,
    transport::{self, Cdp},
};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_FILE: AtomicU64 = AtomicU64::new(1);
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn list_tabs(browser: &Browser) -> Result<Vec<Value>, String> {
    let value = transport::http_get(browser.port, "/json/list")?;
    Ok(value
        .as_array()
        .ok_or("Invalid DevTools target list")?
        .iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| {
            Some(Value::object([
                ("tab_id", Value::string(target.get("id")?.as_str()?)),
                ("title", target.get("title").cloned().unwrap_or(Value::Null)),
                ("url", target.get("url").cloned().unwrap_or(Value::Null)),
            ]))
        })
        .collect())
}

pub(super) fn tab<'a>(
    browser: &'a mut Browser,
    id: &str,
    cancellation: &Cancellation,
) -> Result<&'a mut Cdp, String> {
    if !browser.tabs.get(id).is_some_and(Cdp::usable) {
        let targets = transport::http_get(browser.port, "/json/list")?;
        let target = targets
            .as_array()
            .and_then(|targets| {
                targets
                    .iter()
                    .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
            })
            .ok_or("Browser tab not found")?;
        let url = target
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .ok_or("Browser tab has no DevTools endpoint")?;
        let mut connection = Cdp::connect(url)?;
        // A recreated connection continues the tab's event log and viewport.
        if let Some(previous) = browser.tabs.remove(id) {
            connection.events = previous.events;
            connection
                .events
                .gap("DevTools connection was recreated; events in between were not captured");
        }
        let viewport = browser.viewports.get(id).copied();
        let mut prepare = || {
            for domain in ["Page", "Runtime", "Network"] {
                connection.call(
                    &format!("{domain}.enable"),
                    empty(),
                    cancellation,
                    DEFAULT_TIMEOUT,
                )?;
            }
            viewport.map_or(Ok(()), |size| {
                super::viewport::set(&mut connection, size, cancellation, DEFAULT_TIMEOUT)
            })
        };
        let prepared = prepare();
        if prepared.is_err() {
            connection.invalidate();
        }
        browser.tabs.insert(id.to_string(), connection);
        prepared?;
    }
    browser
        .tabs
        .get_mut(id)
        .ok_or("Browser tab connection unavailable".into())
}

fn selected_tab(browser: &Browser, arguments: &Value) -> Result<String, String> {
    if let Some(id) = arguments.get("tab_id").and_then(Value::as_str) {
        return Ok(id.into());
    }
    list_tabs(browser)?
        .first()
        .and_then(|t| t.get("tab_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("Browser has no page tab".into())
}

pub(super) fn navigate(
    browser: &mut Browser,
    id: &str,
    url: &str,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url == "about:blank") {
        return Err("Browser URL must use http or https".into());
    }
    let connection = tab(browser, id, cancellation)?;
    let result = connection.call(
        "Page.navigate",
        Value::object([("url", Value::string(url))]),
        cancellation,
        DEFAULT_TIMEOUT,
    )?;
    if result.get("errorText").is_some() {
        return Err(format!("Navigation failed: {}", result.encode()));
    }
    Ok(Value::object([
        ("tab_id", Value::string(id)),
        ("url", Value::string(url)),
    ]))
}

pub(super) fn execute(
    browser: &mut Browser,
    area: &Area,
    action: &str,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    if action == "tabs" {
        return Ok(Value::object([
            ("browser_id", Value::string(&browser.id)),
            ("tabs", Value::Array(list_tabs(browser)?)),
        ]));
    }
    let id = selected_tab(browser, arguments)?;
    if action == "close" {
        let version = transport::http_get(browser.port, "/json/version")?;
        let url = version
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .ok_or("Browser DevTools endpoint unavailable")?;
        let mut connection = Cdp::connect(url)?;
        let result = connection.call(
            "Target.closeTarget",
            Value::object([("targetId", Value::string(&id))]),
            cancellation,
            DEFAULT_TIMEOUT,
        )?;
        if result.get("success") != Some(&Value::Bool(true)) {
            return Err("Browser tab did not close".into());
        }
        browser.tabs.remove(&id);
        browser.viewports.remove(&id);
        return Ok(Value::object([("closed", Value::Bool(true))]));
    }
    if action == "navigate" {
        return navigate(browser, &id, required(arguments, "url")?, cancellation);
    }
    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(Value::as_usize)
        .unwrap_or(15000);
    if timeout_ms == 0 {
        return Err("browser timeout_ms must be positive".into());
    }
    let timeout = Duration::from_millis(timeout_ms as u64);
    let mut viewport = None;
    if action == "screenshot"
        && let Some(size) = super::viewport::requested(arguments)?
    {
        viewport = Some(super::viewport::apply(
            browser,
            &id,
            size,
            cancellation,
            timeout,
        )?);
    }
    let connection = tab(browser, &id, cancellation)?;
    match action {
        "cdp" => {
            let method = required(arguments, "method")?;
            let params = arguments.get("params").cloned().unwrap_or_else(empty);
            Ok(Value::object([(
                "result",
                connection.call(method, params, cancellation, timeout)?,
            )]))
        }
        "evaluate" => Ok(Value::object([(
            "value",
            evaluate(
                connection,
                required(arguments, "expression")?,
                cancellation,
                timeout,
            )?,
        )])),
        "snapshot" => super::snapshot::capture(connection, area, &id, cancellation),
        "click" => {
            let element = element_expr(required(arguments, "selector")?);
            let script = format!(
                "(()=>{{const e={element};if(!e)return null;e.scrollIntoView({{block:'center',inline:'center'}});const r=e.getBoundingClientRect(),x=r.left+r.width/2,y=r.top+r.height/2,h=document.elementFromPoint(x,y);return {{x,y,visible:r.width>0&&r.height>0&&x>=0&&x<innerWidth&&y>=0&&y<innerHeight&&(h===e||e.contains(h))}}}})()"
            );
            let point = evaluate(connection, &script, cancellation, timeout)?;
            if point == Value::Null || point.get("visible") != Some(&Value::Bool(true)) {
                return Err("Browser element is missing or not visible".into());
            }
            let x = number(&point, "x")?;
            let y = number(&point, "y")?;
            for kind in ["mousePressed", "mouseReleased"] {
                connection.call(
                    "Input.dispatchMouseEvent",
                    Value::object([
                        ("type", Value::string(kind)),
                        ("x", Value::Number(x.to_string())),
                        ("y", Value::Number(y.to_string())),
                        ("button", Value::string("left")),
                        ("clickCount", Value::number(1)),
                    ]),
                    cancellation,
                    timeout,
                )?;
            }
            Ok(Value::object([
                ("clicked", Value::Bool(true)),
                ("tab_id", Value::string(id)),
            ]))
        }
        "fill" => {
            let element = element_expr(required(arguments, "selector")?);
            let text = required(arguments, "text")?;
            let script = format!(
                "(()=>{{const e={element};if(!e)return false;e.focus();if(!('value' in e))return false;e.value='';e.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()"
            );
            if evaluate(connection, &script, cancellation, timeout)? != Value::Bool(true) {
                return Err("Browser input element not found".into());
            }
            connection.call(
                "Input.insertText",
                Value::object([("text", Value::string(text))]),
                cancellation,
                timeout,
            )?;
            Ok(Value::object([
                ("filled", Value::Bool(true)),
                ("tab_id", Value::string(id)),
            ]))
        }
        "press" => {
            if let Some(target) = arguments.get("selector").and_then(Value::as_str) {
                let script = format!(
                    "(()=>{{const e={};if(!e)return false;e.focus();return true}})()",
                    element_expr(target)
                );
                if evaluate(connection, &script, cancellation, timeout)? != Value::Bool(true) {
                    return Err("Browser element not found".into());
                }
            }
            let key = required(arguments, "key")?;
            let code = match key {
                "Enter" => 13,
                "Tab" => 9,
                "Escape" => 27,
                "Backspace" => 8,
                "Delete" => 46,
                "ArrowLeft" => 37,
                "ArrowUp" => 38,
                "ArrowRight" => 39,
                "ArrowDown" => 40,
                _ => 0,
            };
            for kind in ["keyDown", "keyUp"] {
                connection.call(
                    "Input.dispatchKeyEvent",
                    Value::object([
                        ("type", Value::string(kind)),
                        ("key", Value::string(key)),
                        ("code", Value::string(key)),
                        ("windowsVirtualKeyCode", Value::number(code)),
                    ]),
                    cancellation,
                    timeout,
                )?;
            }
            Ok(Value::object([
                ("pressed", Value::string(key)),
                ("tab_id", Value::string(id)),
            ]))
        }
        "scroll" => {
            let x = arguments.get("x").cloned().unwrap_or(Value::number(400));
            let y = arguments.get("y").cloned().unwrap_or(Value::number(300));
            let delta = arguments
                .get("delta_y")
                .cloned()
                .unwrap_or(Value::number(600));
            connection.call(
                "Input.dispatchMouseEvent",
                Value::object([
                    ("type", Value::string("mouseWheel")),
                    ("x", x),
                    ("y", y),
                    ("deltaX", Value::number(0)),
                    ("deltaY", delta),
                ]),
                cancellation,
                timeout,
            )?;
            Ok(Value::object([
                ("scrolled", Value::Bool(true)),
                ("tab_id", Value::string(id)),
            ]))
        }
        "wait" => super::wait::run(connection, &id, arguments, timeout, cancellation),
        "screenshot" => {
            let result = connection.call(
                "Page.captureScreenshot",
                Value::object([
                    ("format", Value::string("png")),
                    ("captureBeyondViewport", Value::Bool(false)),
                ]),
                cancellation,
                timeout,
            )?;
            let data = result
                .get("data")
                .and_then(Value::as_str)
                .ok_or("Screenshot data unavailable")?;
            let bytes = crate::attachments::base64::decode(data)?;
            let (path, relative) = output_path(area, "png")?;
            fs::write(path, bytes).map_err(|e| e.to_string())?;
            let mut result = Value::object([
                ("tab_id", Value::string(id)),
                ("screenshot_path", Value::string(relative)),
            ]);
            if let (Some(viewport), Value::Object(fields)) = (viewport, &mut result) {
                fields.insert("viewport".into(), viewport);
            }
            Ok(result)
        }
        "logs" => {
            let filter = logs::Filter::parse(arguments.get("filter").and_then(Value::as_str))?;
            let limit = match arguments.get("limit") {
                None => logs::DEFAULT_LIMIT,
                Some(limit) => limit
                    .as_usize()
                    .filter(|n| (1..=logs::MAX_LIMIT).contains(n))
                    .ok_or(format!(
                        "browser logs limit must be an integer from 1 to {}",
                        logs::MAX_LIMIT
                    ))?,
            };
            // Receives the events queued since the previous DevTools call.
            connection.call(
                "Runtime.evaluate",
                Value::object([("expression", Value::string("void 0"))]),
                cancellation,
                timeout,
            )?;
            let page = connection.events.page(
                filter,
                limit,
                arguments.get("cursor").and_then(Value::as_str),
            )?;
            let mut lines = String::new();
            for record in &page.records {
                lines.push_str(&record.encode());
                lines.push('\n');
            }
            let (path, relative) = output_path(area, "jsonl")?;
            fs::write(path, lines).map_err(|e| e.to_string())?;
            Ok(Value::object([
                ("tab_id", Value::string(id)),
                ("entries", Value::Array(page.entries)),
                ("next_cursor", Value::string(page.next_cursor)),
                ("more", Value::Bool(page.more)),
                ("missed", Value::number(page.missed)),
                ("logs_path", Value::string(relative)),
            ]))
        }
        _ => Err(format!("Unknown browser action: {action}")),
    }
}

pub(super) fn evaluate(
    connection: &mut Cdp,
    expression: &str,
    cancellation: &Cancellation,
    timeout: Duration,
) -> Result<Value, String> {
    let result = connection.call(
        "Runtime.evaluate",
        Value::object([
            ("expression", Value::string(expression)),
            ("returnByValue", Value::Bool(true)),
            ("awaitPromise", Value::Bool(true)),
        ]),
        cancellation,
        timeout,
    )?;
    if let Some(exception) = result.get("exceptionDetails") {
        return Err(format!("Browser JavaScript error: {}", exception.encode()));
    }
    Ok(result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("browser {key} is required"))
}
fn empty() -> Value {
    Value::object([])
}
pub(super) fn element_expr(input: &str) -> String {
    if let Some(digits) = input.strip_prefix('@')
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
    {
        return format!(
            "window[Symbol.for('jecode.browser.refs')]?.[{}]",
            digits.parse::<usize>().unwrap_or(0).saturating_sub(1)
        );
    }
    format!("document.querySelector({})", Value::string(input).encode())
}
fn number(value: &Value, key: &str) -> Result<f64, String> {
    match value.get(key) {
        Some(Value::Number(n)) => n.parse().map_err(|_| format!("Invalid {key} coordinate")),
        _ => Err(format!("Missing {key} coordinate")),
    }
}
pub(super) fn output_path(
    area: &Area,
    extension: &str,
) -> Result<(std::path::PathBuf, String), String> {
    let name = format!(
        "capture-{}-{}.{}",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed),
        extension
    );
    let (_, path) = area.file(&format!("browser/{name}"), true)?;
    Ok((path, format!("tmp:browser/{name}")))
}
