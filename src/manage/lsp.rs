//! `jecode lsp`: the entries Settings > LSP manages, through the same store
//! and check. Listing starts nothing; `check` starts a temporary instance
//! because it is asked to. Enabled servers run as the user, independently of
//! the Read, Workspace and Full levels.

use super::{Context, Report, confirm};
use crate::cancel::Cancellation;
use crate::cli::{Failure, Manage, usage};
use crate::json::Value;
use crate::lsp::{CheckFailure, Profile, Store};
use crate::redact::Redactor;

pub fn run(context: &Context, command: &Manage) -> Result<Report, Failure> {
    let store = Store::new(context.home.clone());
    let profiles = store.load()?;
    let redactor = Redactor::empty();
    for profile in &profiles {
        profile.protect(&redactor);
    }
    let report = match command.action {
        "list" => list(&profiles),
        "show" => {
            let profile = find(&profiles, &command.operands[0])?;
            Ok(Report::new(value(profile), detail(profile)))
        }
        "add" => add(
            &store,
            &profiles,
            &command.operands[0],
            command.value("executable"),
        ),
        "check" => check(&store, find(&profiles, &command.operands[0])?),
        "enable" | "disable" => {
            let profile = find(&profiles, &command.operands[0])?;
            let enabled = command.action == "enable";
            store.set_enabled(&profile.id, enabled)?;
            Ok(Report::new(
                Value::object([
                    ("id", Value::string(&profile.id)),
                    ("enabled", Value::Bool(enabled)),
                ]),
                if enabled {
                    format!(
                        "{} turned on · starts on a matching model request",
                        profile.name
                    )
                } else {
                    format!("{} turned off", profile.name)
                },
            ))
        }
        "remove" => {
            let profile = find(&profiles, &command.operands[0])?;
            if !confirm(
                command.flag("yes"),
                &format!("Remove language server {}?", profile.name),
                "remove",
            )? {
                return Err("Cancelled; the server entry was kept".into());
            }
            store.remove(&profile.id)?;
            Ok(Report::new(
                Value::object([
                    ("id", Value::string(&profile.id)),
                    ("removed", Value::Bool(true)),
                ]),
                format!("Removed {}", profile.name),
            ))
        }
        action => unreachable!("lsp {action}"),
    }?;
    Ok(Report::new(
        redactor.value(&report.value),
        redactor.text(&report.text),
    ))
}

fn find<'a>(profiles: &'a [Profile], target: &str) -> Result<&'a Profile, Failure> {
    profiles
        .iter()
        .find(|profile| profile.id == target || profile.name.eq_ignore_ascii_case(target))
        .ok_or_else(|| {
            Failure::Error(format!(
                "No language server named {target}; list them with jecode lsp list"
            ))
        })
}

fn check_state(profile: &Profile) -> String {
    let fingerprint = profile.analysis_fingerprint();
    match (&profile.last_check, &profile.last_check_failure) {
        (Some(check), _) if check.fingerprint == fingerprint => "checked".into(),
        (_, Some(failure)) if failure.fingerprint == fingerprint => {
            format!("check failed: {}", failure.detail)
        }
        _ => "not checked".into(),
    }
}

fn value(profile: &Profile) -> Value {
    Value::object([
        ("id", Value::string(&profile.id)),
        ("name", Value::string(&profile.name)),
        ("enabled", Value::Bool(profile.enabled)),
        ("executable", Value::string(&profile.executable)),
        (
            "arguments",
            Value::Array(profile.arguments.iter().map(Value::string).collect()),
        ),
        (
            "environment",
            Value::Array(profile.environment.keys().map(Value::string).collect()),
        ),
        (
            "languages",
            Value::Array(profile.languages.iter().map(Value::string).collect()),
        ),
        (
            "extensions",
            Value::entries(
                profile
                    .extensions
                    .iter()
                    .map(|(extension, language)| (extension.clone(), Value::string(language))),
            ),
        ),
        (
            "root_markers",
            Value::Array(profile.root_markers.iter().map(Value::string).collect()),
        ),
        (
            "root",
            Value::optional(
                profile
                    .root_override
                    .as_ref()
                    .map(|root| root.to_string_lossy().into_owned()),
            ),
        ),
        (
            "request_timeout_ms",
            Value::number(profile.request_timeout_ms),
        ),
        ("check", Value::string(check_state(profile))),
    ])
}

fn detail(profile: &Profile) -> String {
    let extensions = profile
        .extensions
        .iter()
        .map(|(extension, language)| format!(".{extension} {language}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} ({})\nState:       {}\nExecutable:  {} {}\nEnvironment: {}\nFiles:       {extensions}\nRoot:        {}\nTimeout:     {} ms\nCheck:       {}",
        profile.name,
        profile.id,
        if profile.enabled { "on" } else { "off" },
        profile.executable,
        profile.arguments.join(" "),
        if profile.environment.is_empty() {
            "none".to_owned()
        } else {
            profile
                .environment
                .keys()
                .map(|name| format!("{name}=••••"))
                .collect::<Vec<_>>()
                .join(" ")
        },
        profile.root_override.as_ref().map_or_else(
            || format!("nearest of {}", profile.root_markers.join(", ")),
            |root| root.display().to_string()
        ),
        profile.request_timeout_ms,
        check_state(profile)
    )
}

fn list(profiles: &[Profile]) -> Result<Report, Failure> {
    let mut text = String::new();
    for profile in profiles {
        text.push_str(&format!(
            "{} ({}) · {} · {} · {}\n",
            profile.name,
            profile.id,
            if profile.enabled { "on" } else { "off" },
            profile.executable,
            check_state(profile)
        ));
    }
    if profiles.is_empty() {
        text.push_str("No language servers. Add one with jecode lsp add rust-analyzer|clangd.\n");
    }
    text.push_str("Listing starts nothing; jecode lsp check SERVER starts a temporary instance.");
    Ok(Report::new(
        Value::object([(
            "servers",
            Value::Array(profiles.iter().map(value).collect()),
        )]),
        text,
    ))
}

fn add(
    store: &Store,
    profiles: &[Profile],
    preset: &str,
    executable: Option<&str>,
) -> Result<Report, Failure> {
    let mut profile = match preset {
        "rust-analyzer" | "rust" => Profile::draft_guided_rust(),
        "clangd" | "cpp" => Profile::draft_guided_cpp(),
        _ => {
            return Err(usage(format!(
                "unknown preset '{preset}'; use rust-analyzer or clangd. Custom servers use Settings > LSP or lsp.json"
            )));
        }
    };
    if let Some(executable) = executable {
        profile.executable = executable.to_owned();
    }
    if profiles
        .iter()
        .any(|other| other.name.eq_ignore_ascii_case(&profile.name))
    {
        return Err(Failure::Error(format!(
            "A server named {} already exists; see jecode lsp show {}",
            profile.name, profile.name
        )));
    }
    profile.validate().map_err(usage)?;
    store.save(profile.clone())?;
    Ok(Report::new(
        value(&profile),
        format!(
            "Saved {} ({}) · on · starts on the first matching model request; jecode lsp check {} tries it now",
            profile.name, profile.id, profile.id
        ),
    ))
}

fn check(store: &Store, profile: &Profile) -> Result<Report, Failure> {
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    eprintln!("Starting, initializing and stopping {}…", profile.name);
    let fingerprint = profile.analysis_fingerprint();
    match crate::lsp::check(profile, &workspace, &Cancellation::default()) {
        Ok(observation) => {
            let root = observation.root.clone();
            store.record_check(&profile.id, Ok(observation))?;
            Ok(Report::new(
                Value::object([
                    ("id", Value::string(&profile.id)),
                    ("check", Value::string("checked")),
                    ("root", Value::string(root.to_string_lossy())),
                ]),
                format!(
                    "{} initialized and stopped · root {}",
                    profile.name,
                    root.display()
                ),
            ))
        }
        Err(error) => {
            let redactor = Redactor::empty();
            profile.protect(&redactor);
            let error = redactor.text(&error);
            store.record_check(
                &profile.id,
                Err(CheckFailure {
                    detail: error.clone(),
                    fingerprint,
                }),
            )?;
            Err(Failure::Error(format!(
                "{} check failed: {error}",
                profile.name
            )))
        }
    }
}
