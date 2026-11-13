//! `jecode doctor`: read-only diagnostics. It reads files only: no model
//! request, sign-in, token refresh, language server, network or write.

use super::{Context, Report};
use crate::cli::Failure;
use crate::json::Value;
use std::io::IsTerminal;

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Ok,
    Warning,
    Error,
}

impl Status {
    fn id(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

fn check(name: &'static str, status: Status, detail: impl Into<String>) -> Check {
    Check {
        name,
        status,
        detail: detail.into(),
    }
}

fn checks(context: &Context) -> Vec<Check> {
    let mut checks = Vec::new();
    checks.push(check(
        "version",
        Status::Ok,
        crate::identity::version_line(),
    ));
    checks.push(match crate::install::provenance::running() {
        Ok(owner) => check(
            "installation",
            if matches!(owner, crate::install::provenance::Owner::Unknown { .. }) {
                Status::Warning
            } else {
                Status::Ok
            },
            format!(
                "{}; {}",
                owner.describe(),
                if matches!(
                    owner,
                    crate::install::provenance::Owner::Unknown { .. }
                        | crate::install::provenance::Owner::NpmProject { .. }
                ) {
                    "jecode update and uninstall do not manage it"
                } else {
                    "jecode update and uninstall manage it"
                }
            ),
        ),
        Err(error) => check("installation", Status::Warning, error),
    });
    checks.push(check(
        "home",
        if context.home.is_dir() {
            Status::Ok
        } else {
            Status::Warning
        },
        if context.home.is_dir() {
            context.home.display().to_string()
        } else {
            format!("{} does not exist yet", context.home.display())
        },
    ));
    let settings = context.store.load();
    checks.push(match &settings {
        Ok(Some(settings)) => check(
            "configuration",
            Status::Ok,
            format!(
                "{} · default {} / {} · effort {} · permissions {}",
                context.store.path().display(),
                settings.backend_id,
                settings.model,
                settings.effort.name(),
                settings.default_permissions.id()
            ),
        ),
        Ok(None) => check(
            "configuration",
            Status::Warning,
            format!(
                "{} not found; run jecode setup",
                context.store.path().display()
            ),
        ),
        Err(error) => check("configuration", Status::Error, error.clone()),
    });
    match super::providers::statuses(context) {
        Ok(rows) => {
            let ready: Vec<String> = rows
                .iter()
                .filter(|row| row.get("ready") == Some(&Value::Bool(true)))
                .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            let default = rows
                .iter()
                .find(|row| row.get("default") == Some(&Value::Bool(true)));
            let default_ready =
                default.is_some_and(|row| row.get("ready") == Some(&Value::Bool(true)));
            checks.push(check(
                "providers",
                if matches!(settings, Ok(Some(_))) && !default_ready {
                    Status::Error
                } else if ready.is_empty() {
                    Status::Warning
                } else {
                    Status::Ok
                },
                if ready.is_empty() {
                    "no provider has saved credentials; use jecode providers login".into()
                } else if matches!(settings, Ok(Some(_))) && !default_ready {
                    format!(
                        "the default provider has no credentials; ready: {}",
                        ready.join(", ")
                    )
                } else {
                    format!("ready: {}", ready.join(", "))
                },
            ));
        }
        Err(error) => checks.push(check("providers", Status::Error, error)),
    }
    checks.push(match crate::mcp::Store::new(context.home.clone()).list() {
        Ok(connections) => check(
            "mcp",
            Status::Ok,
            format!(
                "{} connections, {} enabled · not contacted",
                connections.len(),
                connections.iter().filter(|item| item.enabled).count()
            ),
        ),
        Err(error) => check("mcp", Status::Error, error.message),
    });
    checks.push(match crate::lsp::Store::new(context.home.clone()).load() {
        Ok(servers) => check(
            "lsp",
            Status::Ok,
            format!(
                "{} servers, {} on · none started",
                servers.len(),
                servers.iter().filter(|server| server.enabled).count()
            ),
        ),
        Err(error) => check("lsp", Status::Error, error),
    });
    let mut library = crate::skills::Library::at(Some(context.home.join("skills")));
    library.refresh();
    checks.push(check(
        "skills",
        if library.warnings().is_empty() {
            Status::Ok
        } else {
            Status::Warning
        },
        format!(
            "{} skills{}",
            library.list().len(),
            if library.warnings().is_empty() {
                String::new()
            } else {
                format!(" · {}", library.warnings().join(" · "))
            }
        ),
    ));
    let directory = std::env::current_dir();
    checks.push(
        match directory
            .map_err(|error| error.to_string())
            .and_then(|directory| crate::sessions::Store::new(context.home.clone(), &directory))
            .and_then(|store| store.list_read_only())
        {
            Ok(listing) => check(
                "sessions",
                if listing.warnings.is_empty() {
                    Status::Ok
                } else {
                    Status::Warning
                },
                format!(
                    "{} saved in this folder{}",
                    listing.sessions.len(),
                    if listing.warnings.is_empty() {
                        String::new()
                    } else {
                        format!(" · {} left unchanged", listing.warnings.len())
                    }
                ),
            ),
            Err(error) => check("sessions", Status::Warning, error),
        },
    );
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let dumb = std::env::var("TERM").is_ok_and(|term| term == "dumb");
    checks.push(check(
        "terminal",
        Status::Ok,
        if terminal && !dumb {
            "interactive: jecode opens the TUI".to_owned()
        } else {
            "not interactive: use a prompt, a pipe or --plain".to_owned()
        },
    ));
    checks
}

pub fn run(json: bool) -> Result<(), Failure> {
    let context = Context::discover()?;
    let checks = checks(&context);
    let errors = checks
        .iter()
        .filter(|check| check.status == Status::Error)
        .count();
    let mut text = String::new();
    for check in &checks {
        text.push_str(&format!(
            "{:<7} {:<14} {}\n",
            check.status.id(),
            check.name,
            check.detail
        ));
    }
    text.push_str(&format!(
        "{errors} error{} · nothing was changed",
        if errors == 1 { "" } else { "s" }
    ));
    let report = Report::new(
        Value::object([
            ("ok", Value::Bool(errors == 0)),
            (
                "checks",
                Value::Array(
                    checks
                        .iter()
                        .map(|check| {
                            Value::object([
                                ("name", Value::string(check.name)),
                                ("status", Value::string(check.status.id())),
                                ("detail", Value::string(&check.detail)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        text,
    );
    report.print(json, &mut std::io::stdout().lock())?;
    if errors > 0 {
        return Err(Failure::Error(format!(
            "jecode doctor found {errors} error{}",
            if errors == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}
