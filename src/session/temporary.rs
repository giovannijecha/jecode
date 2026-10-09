use crate::agent::Agent;
use std::io::Write;

pub struct Report {
    pub message: String,
    pub details: Vec<(String, String)>,
}

pub fn run(agent: &mut Agent, arguments: &str) -> Result<Report, String> {
    if arguments.is_empty() {
        let info = agent.temporary_info()?;
        Ok(Report {
            message: "Session temporary files".into(),
            details: vec![
                ("path".into(), info.path),
                ("files".into(), info.files.to_string()),
                ("directories".into(), info.directories.to_string()),
                ("size".into(), format!("{} bytes", info.bytes)),
                (
                    "retention".into(),
                    "Kept until /tmp clean, including after resume".into(),
                ),
            ],
        })
    } else if arguments.eq_ignore_ascii_case("clean") {
        let removed = agent.clean_temporary()?;
        Ok(Report {
            message: format!(
                "Cleared {} temporary files and {} directories ({} bytes)",
                removed.files, removed.directories, removed.bytes
            ),
            details: vec![],
        })
    } else {
        Err("Usage: /tmp [clean]. Cleanup affects only this session's temporary files.".into())
    }
}

pub(super) fn print(
    agent: &mut Agent,
    arguments: &str,
    output: &mut impl Write,
) -> Result<(), String> {
    let report = run(agent, arguments)?;
    agent.record_local_details(
        if arguments.is_empty() {
            "/tmp"
        } else {
            "/tmp clean"
        },
        &report.message,
        "notice",
        &report.details,
    );
    writeln!(output, "{}", agent.redact(&report.message)).map_err(|error| error.to_string())?;
    let width = report
        .details
        .iter()
        .map(|(key, _)| key.len())
        .max()
        .unwrap_or(0);
    for (key, value) in report.details {
        writeln!(output, "{key:width$}  {}", agent.redact(&value))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
