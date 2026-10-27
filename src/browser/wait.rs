//! Polls a selector or text condition within the caller's time budget.
use super::{
    actions::{element_expr, evaluate},
    transport::{self, Cdp},
};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);

struct Condition<'a> {
    tab: &'a str,
    selector: Option<&'a str>,
    text: Option<&'a str>,
    timeout: Duration,
    started: Instant,
}

pub(super) fn run(
    connection: &mut Cdp,
    id: &str,
    arguments: &Value,
    timeout: Duration,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let condition = Condition {
        tab: id,
        selector: arguments.get("selector").and_then(Value::as_str),
        text: arguments.get("text").and_then(Value::as_str),
        timeout,
        started: Instant::now(),
    };
    if condition.selector.is_none() && condition.text.is_none() {
        return Err("wait needs selector or text".into());
    }
    // Each check reports the URL, so a timeout needs no call after expiry.
    let script = format!(
        "(()=>{{const e={};return {{element:!!e,found:!!e&&{},url:location.href}}}})()",
        condition
            .selector
            .map_or("document.body".into(), element_expr),
        condition
            .text
            .map(|t| format!("(e.innerText||'').includes({})", Value::string(t).encode()))
            .unwrap_or("true".into())
    );
    let deadline = condition
        .started
        .checked_add(timeout)
        .ok_or("Browser wait timeout is too large")?;
    let mut last = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match evaluate(connection, &script, cancellation, remaining) {
            Ok(state) if state.get("found") == Some(&Value::Bool(true)) => {
                return Ok(Value::object([
                    ("found", Value::Bool(true)),
                    ("tab_id", Value::string(id)),
                ]));
            }
            Ok(state) => last = Some(state),
            Err(error) if transport::timed_out(&error) => {
                return Err(condition.unmet(last.as_ref(), true));
            }
            Err(error) => return Err(error),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        cancellation.wait(remaining.min(POLL))?;
    }
    Err(condition.unmet(last.as_ref(), false))
}

impl Condition<'_> {
    /// Describes the unmet condition from the last completed check.
    fn unmet(&self, last: Option<&Value>, unanswered: bool) -> String {
        let quote = |s: &str| Value::string(s).encode();
        let url = last
            .and_then(|state| state.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("an unknown URL");
        let reason = match (last, self.selector, self.text) {
            _ if last.is_none() => {
                let target = match (self.selector, self.text) {
                    (Some(selector), Some(text)) => {
                        format!("text {} in selector {}", quote(text), quote(selector))
                    }
                    (Some(selector), None) => format!("selector {}", quote(selector)),
                    (None, Some(text)) => format!("text {} in the page body", quote(text)),
                    (None, None) => unreachable!("wait needs selector or text"),
                };
                format!("the page did not answer the last check for {target} before the deadline")
            }
            (Some(state), Some(selector), _)
                if state.get("element") != Some(&Value::Bool(true)) =>
            {
                format!("no element matched selector {}", quote(selector))
            }
            (_, selector, Some(text)) => format!(
                "text {} not found in {}",
                quote(text),
                selector.map_or("the page body".into(), |s| format!("selector {}", quote(s)))
            ),
            _ => "the page has no body".to_string(),
        };
        let reason = if unanswered && last.is_some() {
            format!(
                "{reason} (last completed check); the page did not answer the final check before the deadline"
            )
        } else {
            reason
        };
        format!(
            "Browser wait timed out after {} ms (elapsed {} ms) on tab {} at {url}: {reason}",
            self.timeout.as_millis(),
            self.started.elapsed().as_millis(),
            self.tab
        )
    }
}
