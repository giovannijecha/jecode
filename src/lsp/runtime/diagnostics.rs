use super::Outcome;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::lsp::Profile;
use crate::lsp::client::{Client, DiagnosticReport};
use crate::lsp::documents::{Document, canonical_uri};
use crate::lsp::locations;
use std::time::Duration;

pub(super) fn diagnostics(
    client: &mut Client,
    document: &Document,
    profile: &Profile,
    cancel: &Cancellation,
    stop: &Cancellation,
    outcome: &mut Outcome,
) -> Result<(), String> {
    if client.supports("textDocument/diagnostic") {
        let previous = client
            .pull_diagnostics
            .get(&document.uri)
            .and_then(|r| r.result_id.as_ref());
        let mut params = std::collections::BTreeMap::from([(
            "textDocument".into(),
            Value::object([("uri", Value::string(&document.uri))]),
        )]);
        if let Some(previous) = previous {
            params.insert("previousResultId".into(), Value::string(previous));
        }
        if let Some(identifier) = client
            .capabilities
            .get("diagnosticProvider")
            .and_then(|v| v.get("identifier"))
            .and_then(Value::as_str)
        {
            params.insert("identifier".into(), Value::string(identifier));
        }
        let response = client.query(
            "textDocument/diagnostic",
            Value::Object(params),
            Duration::from_millis(profile.request_timeout_ms),
            cancel,
            stop,
        )?;
        let report = response
            .get("result")
            .ok_or("LSP diagnostic response has no result")?;
        if report.get("kind").and_then(Value::as_str) != Some("unchanged") {
            let values = report
                .get("items")
                .and_then(Value::as_array)
                .ok_or("Invalid LSP diagnostic report")?
                .to_vec();
            let result_id = report
                .get("resultId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            client.put_report(
                document.uri.clone(),
                DiagnosticReport {
                    values,
                    version: Some(document.version),
                    result_id,
                    generation: client.generation,
                    stale: false,
                },
                false,
            );
        } else if let Some(previous) = client.pull_diagnostics.get_mut(&document.uri) {
            previous.version = Some(document.version);
            previous.stale = false;
        }
        if let Some(Value::Object(related)) = report.get("relatedDocuments") {
            for (uri, related_report) in related {
                let uri = canonical_uri(uri);
                match related_report.get("kind").and_then(Value::as_str) {
                    Some("full") => {
                        if let Some(items) = related_report.get("items").and_then(Value::as_array) {
                            let version = client.docs.get(&uri).map(|doc| doc.version);
                            let result_id = related_report
                                .get("resultId")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            client.put_report(
                                uri.clone(),
                                DiagnosticReport {
                                    values: items.to_vec(),
                                    version,
                                    result_id,
                                    generation: client.generation,
                                    stale: false,
                                },
                                false,
                            );
                        }
                    }
                    Some("unchanged") => {}
                    _ => return Err("Invalid related LSP diagnostic report".into()),
                }
            }
        }
    } else {
        client.drain()?;
    }
    let reports = [
        client.pull_diagnostics.get(&document.uri),
        client.push_diagnostics.get(&document.uri),
    ];
    let reports = reports.into_iter().flatten().collect::<Vec<_>>();
    if reports.is_empty() {
        outcome.freshness = "pending".into();
        outcome.detail = "No document diagnostic report observed yet".into();
    } else {
        let mut unique = std::collections::HashSet::new();
        outcome.diagnostics = reports
            .iter()
            .flat_map(|report| locations::diagnostics(&report.values, document, client.encoding))
            .filter(|value| unique.insert(value.encode()))
            .collect();
        outcome.freshness = if client.sync_gap || client.diagnostic_gap || client.sync_kind == 0 {
            "sync_gap"
        } else if reports.iter().any(|report| {
            report.generation != client.generation
                || report.stale
                || report.version.is_some_and(|v| v != document.version)
        }) {
            "stale"
        } else if reports.iter().any(|report| report.version.is_none()) {
            "unverified"
        } else {
            "current"
        }
        .into();
        outcome.detail = format!(
            "{} diagnostic(s) for this document ({} pull, {} push)",
            outcome.diagnostics.len(),
            client.pull_diagnostics.contains_key(&document.uri) as usize,
            client.push_diagnostics.contains_key(&document.uri) as usize
        );
    }
    Ok(())
}
