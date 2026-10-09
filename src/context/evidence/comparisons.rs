use super::Evidence;
use crate::json::Value;

impl Evidence {
    pub(super) fn reconcile_comparisons(
        &mut self,
        at: usize,
        name: &str,
        arguments: &Value,
        result: &Value,
    ) {
        if name != "protect"
            || arguments.get("action").and_then(Value::as_str) != Some("status")
            || result.get("error").is_some()
            || result.get("cancelled") == Some(&Value::Bool(true))
            || matches!(
                result.get("outcome").and_then(Value::as_str),
                Some("unknown" | "not_started")
            )
            || result.get("file_protections_scope").and_then(Value::as_str) != Some("all")
        {
            return;
        }
        let protections = result
            .get("file_protections")
            .and_then(Value::as_array)
            .unwrap_or(&[]);
        self.uninspected.retain(|entry| {
            if entry.get("state").and_then(Value::as_str) != Some("comparison_incomplete") {
                return true;
            }
            let Some(gap) = entry.get("message").and_then(Value::as_usize) else {
                return true;
            };
            // A later native byte comparison resolves the current uncertainty;
            // it does not turn content inspection or checks into completed work.
            !(gap < at
                && protections.iter().any(|protection| {
                    protection.get("path") == entry.get("path")
                        && protection.get("state").and_then(Value::as_str) == Some("preserved")
                        && protection
                            .get("history_reference")
                            .and_then(Value::as_str)
                            .and_then(|reference| reference.strip_prefix("history:"))
                            .and_then(|index| index.parse::<usize>().ok())
                            .is_some_and(|registered| registered < gap)
                }))
        });
    }
}
