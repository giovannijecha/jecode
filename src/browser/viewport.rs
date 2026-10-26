//! Page viewport dimensions requested by `open` and `screenshot`.
use super::{Browser, actions, transport::Cdp};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::time::Duration;

const MAX: usize = 10_000;

/// The requested CSS viewport, or `None` when neither dimension is given.
pub(super) fn requested(arguments: &Value) -> Result<Option<(u32, u32)>, String> {
    let dimension = |value: &Value| {
        value
            .as_usize()
            .filter(|n| (1..=MAX).contains(n))
            .map(|n| n as u32)
            .ok_or_else(|| format!("browser width and height must be integers from 1 to {MAX}"))
    };
    match (arguments.get("width"), arguments.get("height")) {
        (None, None) => Ok(None),
        (Some(width), Some(height)) => Ok(Some((dimension(width)?, dimension(height)?))),
        _ => Err("browser width and height must be supplied together".into()),
    }
}

/// Emulates a page viewport on one tab connection.
pub(super) fn set(
    connection: &mut Cdp,
    (width, height): (u32, u32),
    cancellation: &Cancellation,
    timeout: Duration,
) -> Result<(), String> {
    connection.call(
        "Emulation.setDeviceMetricsOverride",
        Value::object([
            ("width", Value::number(width)),
            ("height", Value::number(height)),
            ("deviceScaleFactor", Value::number(1)),
            ("mobile", Value::Bool(false)),
        ]),
        cancellation,
        timeout,
    )?;
    Ok(())
}

/// Applies a viewport for the tab's lifetime and returns the page's measured
/// `innerWidth` and `innerHeight`.
pub(super) fn apply(
    browser: &mut Browser,
    id: &str,
    size: (u32, u32),
    cancellation: &Cancellation,
    timeout: Duration,
) -> Result<Value, String> {
    set(
        actions::tab(browser, id, cancellation)?,
        size,
        cancellation,
        timeout,
    )?;
    browser.viewports.insert(id.to_string(), size);
    actions::evaluate(
        actions::tab(browser, id, cancellation)?,
        "({width:innerWidth,height:innerHeight})",
        cancellation,
        timeout,
    )
}
