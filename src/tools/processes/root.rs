//! Native facts about a root process that exited before its supervised tree.

use crate::json::Value;
use crate::process::RootExit;

pub(super) fn redacted(root: &RootExit, redactor: &crate::redact::Redactor) -> RootExit {
    let mut root = root.clone();
    if let Some(remaining) = &mut root.remaining {
        for (_, image) in &mut remaining.processes {
            *image = redactor.text(image);
        }
    }
    root
}

/// Remaining processes listed in one result; the count stays exact.
const LISTED: usize = 16;

/// Status fields for a root exit. While the process runs they also say what
/// keeps it open; once it has ended they only record the earlier exit.
pub(super) fn fields(root: &RootExit, running: bool) -> Vec<(String, Value)> {
    let mut fields = vec![(
        "root_process".to_string(),
        Value::object([
            ("pid", Value::number(root.pid)),
            ("status", Value::string("exited")),
            (
                "exit_code",
                root.exit_code.map_or(Value::Null, Value::number),
            ),
            ("exited_after_ms", Value::number(root.elapsed.as_millis())),
        ]),
    )];
    if !running {
        return fields;
    }
    match &root.remaining {
        Some(remaining) => {
            fields.push((
                "waiting_for".into(),
                Value::string("supervised descendant processes"),
            ));
            fields.push((
                "descendants".into(),
                Value::object([
                    ("running", Value::number(remaining.count)),
                    (
                        "processes",
                        Value::Array(
                            remaining
                                .processes
                                .iter()
                                .take(LISTED)
                                .map(|(pid, image)| {
                                    Value::object([
                                        ("pid", Value::number(*pid)),
                                        ("image", Value::string(image)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]),
            ));
        }
        None => fields.push((
            "waiting_for".into(),
            Value::string("command output held open by descendant processes"),
        )),
    }
    fields
}

/// The earlier root exit as one clause of a completion report.
pub(super) fn clause(root: &RootExit) -> String {
    let code = root
        .exit_code
        .map_or_else(|| "no exit code".into(), |code| format!("code {code}"));
    format!(
        "root process {} had exited with {code} after {}s while supervised descendants kept running",
        root.pid,
        root.elapsed.as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Remaining;
    use std::time::Duration;

    fn root(remaining: Option<Remaining>) -> RootExit {
        RootExit {
            pid: 40,
            exit_code: Some(7),
            elapsed: Duration::from_millis(1500),
            remaining,
        }
    }

    #[test]
    fn running_fields_name_the_root_outcome_and_what_keeps_the_tree_open() {
        let windows = root(Some(Remaining {
            count: 20,
            processes: (0..20).map(|pid| (pid, format!("p{pid}.exe"))).collect(),
        }));
        let fields: std::collections::BTreeMap<_, _> = fields(&windows, true).into_iter().collect();
        let process = &fields["root_process"];
        assert_eq!(process.get("exit_code"), Some(&Value::number(7)));
        assert_eq!(process.get("exited_after_ms"), Some(&Value::number(1500)));
        let descendants = &fields["descendants"];
        assert_eq!(descendants.get("running"), Some(&Value::number(20)));
        assert_eq!(
            descendants
                .get("processes")
                .and_then(Value::as_array)
                .map(|processes| processes.len()),
            Some(LISTED)
        );
        let unix: std::collections::BTreeMap<_, _> = fields_of(root(None));
        assert!(unix["waiting_for"].as_str().unwrap().contains("output"));
        assert!(!unix.contains_key("descendants"));
        let ended: Vec<_> = super::fields(&root(None), false);
        assert_eq!(ended.len(), 1, "an ended process keeps only the root facts");
        assert_eq!(
            clause(&root(None)),
            "root process 40 had exited with code 7 after 1s while supervised descendants kept running"
        );
    }

    fn fields_of(root: RootExit) -> std::collections::BTreeMap<String, Value> {
        fields(&root, true).into_iter().collect()
    }
}
