//! Dedicated local Chromium sessions controlled through the DevTools protocol.
mod actions;
mod launch;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod tests;
mod transport;

use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

static NEXT_BROWSER: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct Registry {
    browsers: Mutex<Vec<Browser>>,
}

struct Browser {
    supervisor: Option<JoinHandle<Result<crate::process::Output, String>>>,
    lifetime: Cancellation,
    profile: PathBuf,
    lease: Option<launch::ProfileLease>,
    port: u16,
    tabs: BTreeMap<String, transport::Cdp>,
    id: String,
}

impl Browser {
    fn stop(&mut self) -> Result<(), String> {
        if self.supervisor.is_some()
            && self.port != 0
            && let Ok(version) = transport::http_get(self.port, "/json/version")
            && let Some(url) = version.get("webSocketDebuggerUrl").and_then(Value::as_str)
            && let Ok(mut connection) = transport::Cdp::connect(url)
        {
            let _ = connection.call(
                "Browser.close",
                Value::object([]),
                &Cancellation::default(),
                std::time::Duration::from_secs(2),
            );
        }
        let mut outcome = Ok(());
        if let Some(supervisor) = self.supervisor.take() {
            let until = Instant::now() + Duration::from_secs(2);
            while !supervisor.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(20));
            }
            if !supervisor.is_finished() {
                self.lifetime.cancel();
            }
            outcome = supervisor
                .join()
                .map_err(|_| "Browser process supervisor panicked".to_string())?
                .map(|_| ());
            if outcome.as_ref().is_err_and(|e| e == "Operation cancelled") {
                outcome = Ok(());
            }
        }
        self.tabs.clear();
        self.lease.take();
        self.port = 0;
        outcome
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl Registry {
    pub fn execute(
        &self,
        area: &Area,
        profiles: Option<&Path>,
        arguments: &Value,
        cancellation: &Cancellation,
    ) -> Result<Value, String> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or("browser action is required")?;
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let mut browsers = self
            .browsers
            .lock()
            .map_err(|_| "Browser registry lock failed")?;
        if action == "open" {
            let visible = matches!(arguments.get("visible"), Some(Value::Bool(true)));
            let name = arguments.get("profile").and_then(Value::as_str);
            if name.is_some()
                && browsers
                    .iter()
                    .any(|b| b.profile.file_name().and_then(|s| s.to_str()) == name)
            {
                return Err("Browser profile is already open".into());
            }
            let mut browser = launch::start(area, profiles, name, visible, cancellation)?;
            browser.id = format!(
                "browser-{}-{}",
                std::process::id(),
                NEXT_BROWSER.fetch_add(1, Ordering::Relaxed)
            );
            let id = browser.id.clone();
            let tabs = actions::list_tabs(&browser)?;
            let tab_id = tabs
                .first()
                .and_then(|t| t.get("tab_id"))
                .and_then(Value::as_str)
                .ok_or("Browser has no page target")?
                .to_string();
            if let Some(url) = arguments.get("url").and_then(Value::as_str) {
                actions::navigate(&mut browser, &tab_id, url, cancellation)?;
            }
            browsers.push(browser);
            return Ok(Value::object([
                ("browser_id", Value::string(id)),
                ("tab_id", Value::string(tab_id)),
                (
                    "url",
                    arguments
                        .get("url")
                        .cloned()
                        .unwrap_or(Value::string("about:blank")),
                ),
            ]));
        }
        let selected = if let Some(id) = arguments.get("browser_id").and_then(Value::as_str) {
            browsers.iter().position(|b| b.id == id)
        } else {
            browsers.len().checked_sub(1)
        }
        .ok_or("Browser session not found; call open first")?;
        if action == "stop" {
            let mut browser = browsers.remove(selected);
            browser.stop()?;
            return Ok(Value::object([("closed", Value::Bool(true))]));
        }
        actions::execute(
            &mut browsers[selected],
            area,
            action,
            arguments,
            cancellation,
        )
    }

    pub fn stop_all(&self) {
        if let Ok(mut browsers) = self.browsers.lock() {
            browsers.clear();
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.stop_all();
    }
}
