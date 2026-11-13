//! `jecode providers`: the Settings > Providers routes. API keys bill the
//! provider's API account; ChatGPT and Claude accounts use their plans. The
//! two never stand in for each other, and no secret is printed or accepted
//! as an argument.

use super::{Context, Report, confirm, secret};
use crate::auth::{AccountStore, SignInUpdate};
use crate::backend::{self, Services};
use crate::cancel::Cancellation;
use crate::cli::{Failure, Manage, usage};
use crate::json::Value;
use crate::redact::Redactor;

#[cfg(test)]
mod tests;

pub fn run(context: &Context, command: &Manage) -> Result<Report, Failure> {
    match command.action {
        "list" => list(context),
        "accounts" => accounts(context, provider(&command.operands[0])?),
        "use" => use_account(
            context,
            provider(&command.operands[0])?,
            &command.operands[1],
        ),
        "login" => login(context, provider(&command.operands[0])?),
        "logout" => logout(
            context,
            provider(&command.operands[0])?,
            command.flag("yes"),
            command.value("account"),
        ),
        action => unreachable!("providers {action}"),
    }
}

fn provider(id: &str) -> Result<&'static backend::BackendInfo, Failure> {
    backend::info(id).ok_or_else(|| {
        usage(format!(
            "unknown provider '{id}'; use {}",
            crate::cli::grammar::PROVIDERS.join(", ")
        ))
    })
}

pub fn route(info: &backend::BackendInfo) -> (&'static str, &'static str) {
    match info.id {
        "openai" => ("account", "ChatGPT plan"),
        "anthropic" => ("account", "Claude plan"),
        _ => ("api-key", "API billing"),
    }
}

fn account_store(context: &Context, info: &backend::BackendInfo) -> Result<AccountStore, Failure> {
    if info.uses_api_key() {
        return Err(usage(format!(
            "{} uses an API key, not saved accounts",
            info.name
        )));
    }
    Ok(AccountStore::new(context.home.clone(), info.id))
}

fn accounts(context: &Context, info: &backend::BackendInfo) -> Result<Report, Failure> {
    let store = account_store(context, info)?;
    let saved = store.accounts()?;
    let active = store.active()?.map(|session| session.id().to_owned());
    let mut text = String::new();
    let rows = saved
        .iter()
        .map(|account| {
            let current = active.as_deref() == Some(&account.id);
            text.push_str(&format!(
                "{}  {}{}{}\n",
                account.id,
                account.label,
                if account.detail.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", account.detail)
                },
                if current {
                    " · active"
                } else if account.enabled {
                    " · available"
                } else {
                    " · signed out"
                }
            ));
            Value::object([
                ("id", Value::string(&account.id)),
                ("label", Value::string(&account.label)),
                ("detail", Value::string(&account.detail)),
                ("enabled", Value::Bool(account.enabled)),
                ("active", Value::Bool(current)),
            ])
        })
        .collect();
    if saved.is_empty() {
        text.push_str(&format!("No saved {} accounts.\n", info.name));
    }
    text.push_str(&format!(
        "Select one with jecode providers use {} ACCOUNT_ID.",
        info.id
    ));
    let report = Report::new(
        Value::object([
            ("provider", Value::string(info.id)),
            ("accounts", Value::Array(rows)),
        ]),
        text,
    );
    Ok(report.redact(&context.redactor()?))
}

fn use_account(
    context: &Context,
    info: &backend::BackendInfo,
    id: &str,
) -> Result<Report, Failure> {
    let store = account_store(context, info)?;
    let redactor = context.redactor()?;
    let account = store.session(id)?.account()?;
    store.activate(id)?;
    let report = Report::new(
        Value::object([
            ("provider", Value::string(info.id)),
            ("account", Value::string(&account.id)),
            ("label", Value::string(&account.label)),
            ("active", Value::Bool(true)),
        ]),
        format!(
            "Using {} ({}) · default provider and model unchanged",
            account.label, info.name
        ),
    );
    Ok(report.redact(&redactor))
}

/// Each provider's readiness from saved files only: no request, refresh or sign-in.
pub fn statuses(context: &Context) -> Result<Vec<Value>, String> {
    let settings = context.store.load()?;
    let mut rows = Vec::new();
    for info in backend::available() {
        let (kind, billing) = route(info);
        let default = settings
            .as_ref()
            .is_some_and(|settings| settings.backend_id == info.id);
        let mut fields = vec![
            ("id", Value::string(info.id)),
            ("name", Value::string(info.name)),
            ("route", Value::string(kind)),
            ("billing", Value::string(billing)),
            ("default", Value::Bool(default)),
        ];
        if info.uses_api_key() {
            let saved = settings
                .as_ref()
                .and_then(|settings| settings.credential_for(info.id))
                .is_some_and(|key| !key.is_empty());
            fields.push(("ready", Value::Bool(saved)));
            fields.push((
                "status",
                Value::string(if saved { "key saved" } else { "no key" }),
            ));
        } else {
            let store = AccountStore::new(context.home.clone(), info.id);
            let accounts = store.accounts()?;
            let active = store.active()?.map(|session| session.id().to_owned());
            let current = accounts
                .iter()
                .find(|account| Some(&account.id) == active.as_ref());
            fields.push(("ready", Value::Bool(current.is_some())));
            fields.push((
                "status",
                Value::string(match current {
                    Some(account) => format!("signed in as {}", account.label),
                    None if accounts.is_empty() => "not signed in".into(),
                    None => format!("{} saved, none active", accounts.len()),
                }),
            ));
            fields.push(("accounts", Value::number(accounts.len())));
        }
        rows.push(Value::entries(fields));
    }
    Ok(rows)
}

fn list(context: &Context) -> Result<Report, Failure> {
    let rows = statuses(context)?;
    let field = |row: &Value, key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut text = String::new();
    for row in &rows {
        text.push_str(&format!(
            "{:<14} {:<14} {:<13} {}{}\n",
            field(row, "id"),
            field(row, "name"),
            field(row, "billing"),
            field(row, "status"),
            if row.get("default") == Some(&Value::Bool(true)) {
                " · default"
            } else {
                ""
            }
        ));
    }
    text.push_str("Save a key or sign in with jecode providers login PROVIDER.");
    Ok(
        Report::new(Value::object([("providers", Value::Array(rows))]), text)
            .redact(&context.redactor()?),
    )
}

fn login(context: &Context, info: &backend::BackendInfo) -> Result<Report, Failure> {
    if !info.uses_api_key() {
        return sign_in(context, info);
    }
    let mut settings = context.store.load()?.ok_or_else(|| {
        Failure::Error(
            "No saved configuration yet. Run jecode setup, which saves the key with a default model."
                .into(),
        )
    })?;
    let key = secret(&format!("{} (hidden): ", info.key_label()))?;
    let redactor = Redactor::new(key.clone());
    let services =
        Services::for_backend(info.id, key.clone()).map_err(|error| redactor.text(&error))?;
    eprintln!("Checking the key with {}...", info.name);
    services
        .check_credential()
        .map_err(|error| redactor.text(&error))?;
    settings.credentials.insert(info.id.into(), key.clone());
    if settings.backend_id == info.id {
        settings.api_key = key;
    }
    context
        .store
        .save(&settings)
        .map_err(|error| redactor.text(&error))?;
    Ok(Report::new(
        Value::object([
            ("provider", Value::string(info.id)),
            ("status", Value::string("key saved")),
        ]),
        format!(
            "{} validated and saved in {} · {}",
            info.key_label(),
            context.store.path().display(),
            route(info).1
        ),
    ))
}

fn sign_in(context: &Context, info: &backend::BackendInfo) -> Result<Report, Failure> {
    let plan = route(info).1;
    if info.id == "anthropic" {
        eprintln!(
            "Anthropic reserves Claude login for its native apps. Use an Anthropic API key for Jecode."
        );
    }
    let store = AccountStore::new(context.home.clone(), info.id);
    let session = store.sign_in(None, Cancellation::default(), &mut |update| {
        match update {
            SignInUpdate::Browser(url) => {
                eprintln!("Continue in your browser:\n{url}")
            }
            SignInUpdate::Waiting => eprintln!("Waiting for sign-in…"),
        }
        Ok(())
    })?;
    store.activate(session.id())?;
    let label = session
        .account()
        .map(|account| account.label)
        .unwrap_or_else(|_| session.id().to_owned());
    Ok(Report::new(
        Value::object([
            ("provider", Value::string(info.id)),
            ("status", Value::string("signed in")),
            ("account", Value::string(&label)),
        ]),
        format!(
            "Signed in to {} ({plan}) as {label} · active account. Default provider and model are unchanged.",
            info.name
        ),
    ))
}

fn logout(
    context: &Context,
    info: &backend::BackendInfo,
    yes: bool,
    account: Option<&str>,
) -> Result<Report, Failure> {
    if info.uses_api_key() {
        if account.is_some() {
            return Err(usage(
                "--account applies only to ChatGPT or Claude accounts",
            ));
        }
        let settings = context.store.load()?;
        let saved = settings
            .as_ref()
            .and_then(|settings| settings.credential_for(info.id))
            .is_some_and(|key| !key.is_empty());
        if !saved {
            return Err(Failure::Error(format!("{} has no saved key", info.name)));
        }
        let default = settings
            .as_ref()
            .is_some_and(|settings| settings.backend_id == info.id);
        let question = if default {
            format!(
                "Remove the saved {}? It belongs to the default provider, which then needs a new key or another default.",
                info.key_label()
            )
        } else {
            format!("Remove the saved {}?", info.key_label())
        };
        if !confirm(yes, &question, "remove")? {
            return Err("Cancelled; the key was kept".into());
        }
        context.store.remove_credential(info.id)?;
        return Ok(Report::new(
            Value::object([
                ("provider", Value::string(info.id)),
                ("status", Value::string("no key")),
            ]),
            format!("{} removed", info.key_label()),
        ));
    }
    let store = account_store(context, info)?;
    let redactor = context.redactor()?;
    let session = match account {
        Some(id) => store.session(id)?,
        None => store
            .active()?
            .ok_or_else(|| Failure::Error(format!("{} has no active account", info.name)))?,
    };
    let label = session
        .account()
        .map(|account| account.label)
        .unwrap_or_else(|_| session.id().to_owned());
    if !confirm(
        yes,
        &redactor.text(&format!(
            "Sign out of {label} ({})? Its saved credentials are removed.",
            info.name
        )),
        "logout",
    )? {
        return Err("Cancelled; still signed in".into());
    }
    let warning = session
        .sign_out(&Cancellation::default())
        .map_err(|error| redactor.text(&error))?;
    let mut text = format!("Signed out of {label} · saved credentials removed");
    if let Some(warning) = &warning {
        text.push_str(&format!("\n{warning}"));
    }
    let report = Report::new(
        Value::object([
            ("provider", Value::string(info.id)),
            ("account", Value::string(session.id())),
            ("status", Value::string("signed out")),
            ("warning", Value::optional(warning)),
        ]),
        text,
    );
    Ok(report.redact(&redactor))
}
