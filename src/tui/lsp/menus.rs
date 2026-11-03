use super::{Panel, Profile};
use crate::json::Value;
use crate::lsp::Status;
use crate::tui::selector::Selector;

pub(super) fn list(profiles: &[Profile], observed: &[Status], settings: bool) -> Selector {
    let mut rows = profiles
        .iter()
        .map(|profile| {
            let state = if !profile.enabled {
                "Off".into()
            } else if let Some(status) = observed.iter().find(|status| status.server == profile.id)
            {
                state_label(&status.state).into()
            } else if let Some(failure) = &profile.last_check_failure {
                if missing_executable(&failure.detail) {
                    "Executable not found".into()
                } else {
                    "Check failed".into()
                }
            } else if profile.last_check.is_some() {
                "Checked · not started".into()
            } else {
                "Not checked · not started".into()
            };
            (profile.name.clone(), state)
        })
        .collect::<Vec<_>>();
    if settings {
        rows.push(("Add server".into(), String::new()));
    }
    let mut menu = Selector::lsp(
        Panel::List,
        "LSP",
        settings.then_some("Settings"),
        "all conversations",
        rows,
    );
    menu.message = if profiles.is_empty() && !settings {
        "No configured servers · open Settings › LSP".into()
    } else {
        format!(
            "{} configured · {} on",
            profiles.len(),
            profiles.iter().filter(|p| p.enabled).count()
        )
    };
    menu
}

pub(super) fn add() -> Selector {
    Selector::lsp(
        Panel::Add,
        "Add language server",
        Some("LSP"),
        "",
        vec![
            ("Rust / rust-analyzer".into(), "Guided profile".into()),
            ("C/C++ / clangd".into(), "Guided profile".into()),
            (
                "Custom server".into(),
                "Any compatible LSP executable".into(),
            ),
        ],
    )
}

pub(super) fn detail(profile: &Profile, observed: Option<&Status>, settings: bool) -> Selector {
    let rows = if settings {
        let mut rows = vec![
            ("Edit".into(), String::new()),
            (
                if profile.enabled {
                    "Turn off"
                } else {
                    "Turn on"
                }
                .into(),
                String::new(),
            ),
            ("Check".into(), "Start a temporary server".into()),
        ];
        if observed.is_some_and(|status| status.state == "restart_suppressed") {
            rows.push((
                "Allow restart".into(),
                "Next request may start a server".into(),
            ));
        }
        rows.push(("Remove".into(), String::new()));
        rows
    } else {
        vec![("Settings detail".into(), String::new())]
    };
    let mut menu = Selector::lsp(
        Panel::Detail,
        &profile.name,
        Some("LSP"),
        "all conversations",
        rows,
    );
    let check = if let Some(failure) = &profile.last_check_failure {
        if missing_executable(&failure.detail) {
            "Executable not found".into()
        } else {
            "Check failed".into()
        }
    } else {
        profile
            .last_check
            .as_ref()
            .map_or("Not checked".into(), |observation| {
                format!(
                    "Checked · {} · {}",
                    observation.executable,
                    observation.root.display()
                )
            })
    };
    let capabilities = profile
        .last_check
        .as_ref()
        .map(|check| capability_facts(&check.capabilities));
    let instance = observed.map_or("Instance · not started".into(), |status| {
        format!(
            "Instance · {} · {}",
            state_label(&status.state),
            status
                .root
                .as_ref()
                .map_or("root not observed".into(), |root| root
                    .display()
                    .to_string())
        )
    });
    menu.message = format!(
        "{} · {check}\n{instance}\n{}Runs as · you · not limited by Read / Workspace / Full",
        if profile.enabled { "On" } else { "Off" },
        capabilities.map_or(String::new(), |facts| format!("{facts}\n"))
    );
    menu
}

pub(super) fn form(profile: &Profile) -> Selector {
    let mut menu = Selector::lsp(
        Panel::Form,
        "Language server",
        Some("LSP"),
        "all conversations",
        vec![
            ("Name".into(), profile.name.clone()),
            ("Executable".into(), profile.executable.clone()),
            ("Languages".into(), associations(profile)),
            (
                "Enabled".into(),
                if profile.enabled { "On" } else { "Off" }.into(),
            ),
            ("Advanced".into(), String::new()),
            ("Check".into(), "Temporary server · does not save".into()),
            ("Save server".into(), String::new()),
        ],
    );
    let check = if let Some(failure) = &profile.last_check_failure {
        if failure.fingerprint == profile.analysis_fingerprint() {
            "Check failed draft"
        } else {
            "Check needed after changes"
        }
    } else {
        profile
            .last_check
            .as_ref()
            .map_or("Not checked", |observation| {
                if observation.fingerprint == profile.analysis_fingerprint() {
                    "Checked draft"
                } else {
                    "Check needed after changes"
                }
            })
    };
    menu.message = format!("{check}\nRuns as · you · not limited by Read / Workspace / Full");
    menu
}

pub(super) fn advanced(profile: &Profile) -> Selector {
    Selector::lsp(
        Panel::Advanced,
        "Advanced",
        Some("Language server"),
        "",
        vec![
            (
                "Arguments".into(),
                format!("{} configured", profile.arguments.len()),
            ),
            (
                "Environment".into(),
                format!("{} configured, values hidden", profile.environment.len()),
            ),
            ("Root markers".into(), profile.root_markers.join(", ")),
            (
                "Root override".into(),
                profile
                    .root_override
                    .as_ref()
                    .map_or("Automatic".into(), |root| root.display().to_string()),
            ),
            (
                "Initialization options".into(),
                if profile.initialization_options == crate::json::Value::Null {
                    "Default"
                } else {
                    "Configured"
                }
                .into(),
            ),
            (
                "Settings".into(),
                if profile.settings == crate::json::Value::object([]) {
                    "Default"
                } else {
                    "Configured"
                }
                .into(),
            ),
            (
                "Request timeout".into(),
                format!("{} ms", profile.request_timeout_ms),
            ),
        ],
    )
}

pub(super) fn associations(profile: &Profile) -> String {
    profile
        .languages
        .iter()
        .map(|language| {
            let extensions = profile
                .extensions
                .iter()
                .filter(|(_, id)| *id == language)
                .map(|(extension, _)| format!(".{extension}"))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{language}: {extensions}")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn capability_facts(capabilities: &Value) -> String {
    let offered = |key: &str| {
        capabilities
            .get(key)
            .is_some_and(|value| value != &Value::Null && value != &Value::Bool(false))
    };
    format!(
        "Definition {} · References {}\nPull {} · Push not observed",
        if offered("definitionProvider") {
            "advertised"
        } else {
            "not advertised"
        },
        if offered("referencesProvider") {
            "advertised"
        } else {
            "not advertised"
        },
        if offered("diagnosticProvider") {
            "advertised"
        } else {
            "not advertised"
        }
    )
}

fn missing_executable(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("not found") || detail.contains("no such file")
}

pub(super) fn state_label(state: &str) -> &str {
    match state {
        "off" => "Off",
        "not_started" => "Not started",
        "starting" => "Starting",
        "ready" => "Ready",
        "busy" => "Request in progress",
        "crashed" | "exited" => "Server exited",
        "restart_suppressed" => "Automatic restart paused",
        "not_found" => "Executable not found",
        "timed_out" => "Timed out",
        "cancelled" => "Cancelled",
        "settings_changed" => "Settings changed",
        "stopped" => "Stopped",
        "configuration_error" => "Configuration error",
        "invalid_input" => "Invalid request",
        "protocol_error" => "Protocol error",
        other => other,
    }
}
