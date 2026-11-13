//! `jecode update` and `jecode uninstall` for the running installation.
//! Its owner comes from where the executable lives (see `provenance`); an
//! unrecognized installation is reported, never guessed at.

pub(crate) mod archive;
mod npm;
pub(crate) mod provenance;
pub(crate) mod registry;
pub(crate) mod sha512;
mod standalone;
mod system;

use crate::cli::Failure;
use crate::json::Value;
use provenance::Owner;
use std::io::Write;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print(json: bool, value: Value, text: &str) -> Result<(), Failure> {
    let mut output = std::io::stdout().lock();
    let rendered = if json {
        format!("{}\n", value.pretty())
    } else {
        format!("{text}\n")
    };
    output
        .write_all(rendered.as_bytes())
        .and_then(|()| output.flush())
        .map_err(|error| Failure::Error(format!("Could not write output: {error}")))
}

fn unmanaged(owner: &Owner) -> Option<String> {
    match owner {
        Owner::NpmProject { project, .. } => Some(format!(
            "This Jecode belongs to the npm project at {}; change it with that project's package manager",
            project.display()
        )),
        Owner::Unknown { binary } => Some(format!(
            "{} was not installed by npm, Volta or the standalone installer, so Jecode does not manage it; replace it the way you installed it",
            binary.display()
        )),
        _ => None,
    }
}

pub fn update(check: bool, json: bool) -> Result<(), Failure> {
    let owner = provenance::running()?;
    if !check && let Some(reason) = unmanaged(&owner) {
        return Err(Failure::Error(reason));
    }
    if matches!(owner, Owner::Standalone { .. }) {
        standalone::supported()?;
    }
    let release = registry::latest()?;
    let available = registry::newer(&release.version, VERSION);
    let mut fields = vec![
        ("installed", Value::string(VERSION)),
        ("latest", Value::string(&release.version)),
        ("update_available", Value::Bool(available)),
        ("installation", owner.value()),
    ];
    if check || !available {
        let text = format!(
            "{}\nInstalled: Jecode {VERSION} · {}\nLatest:    Jecode {} from {}",
            if available {
                "An update is available; run jecode update"
            } else {
                "Jecode is up to date"
            },
            owner.describe(),
            release.version,
            registry::registry()
        );
        fields.push(("updated", Value::Bool(false)));
        return print(json, Value::entries(fields), &text);
    }
    let outcome = match &owner {
        Owner::Standalone { root, binary } => {
            let outcome = standalone::update(root, binary, &release)?;
            (outcome.text, outcome.value)
        }
        Owner::Npm { .. } | Owner::Volta { .. } => {
            let outcome = npm::update(&owner, &release)?;
            (outcome.text, outcome.value)
        }
        _ => unreachable!("unmanaged installations returned above"),
    };
    fields.push(("result", outcome.1));
    print(json, Value::entries(fields), &outcome.0)
}

pub fn uninstall(yes: bool, json: bool) -> Result<(), Failure> {
    let owner = provenance::running()?;
    if let Some(reason) = unmanaged(&owner) {
        return Err(Failure::Error(reason));
    }
    let home = crate::config::Store::discover()?
        .path()
        .parent()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if !crate::manage::confirm(
        yes,
        &format!(
            "Remove the {}?\nYour settings, credentials and conversations in {home} are kept.",
            owner.describe()
        ),
        "uninstall",
    )? {
        return Err(Failure::Error("Cancelled; nothing was removed".into()));
    }
    let (text, value) = match &owner {
        Owner::Standalone { root, binary } => {
            let outcome = standalone::uninstall(root, binary, Some("Environment"))?;
            (outcome.text, outcome.value)
        }
        Owner::Npm { .. } | Owner::Volta { .. } => {
            let outcome = npm::uninstall(&owner)?;
            (outcome.text, outcome.value)
        }
        _ => unreachable!("unmanaged installations returned above"),
    };
    let text = format!(
        "{text}\nYour Jecode home {home} was kept; delete it yourself to remove settings, credentials and conversations."
    );
    let value = match value {
        Value::Object(mut fields) => {
            fields.insert("home_kept".into(), Value::string(&home));
            Value::Object(fields)
        }
        other => other,
    };
    print(json, value, &text)
}
