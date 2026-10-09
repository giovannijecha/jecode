use crate::json::Value;

/// Pending task annotations convey no completion, decision or verification proof.
pub(super) fn normalize(candidate: &mut Value) -> Result<(), String> {
    let Value::Object(fields) = candidate else {
        return Ok(());
    };
    let Some(Value::Array(items)) = fields.get_mut("remaining") else {
        return Ok(());
    };
    for (index, item) in items.iter_mut().enumerate() {
        let Value::Object(fields) = item else {
            continue;
        };
        if let Some(key) = fields
            .keys()
            .find(|key| !matches!(key.as_str(), "description" | "kind"))
        {
            return Err(format!(
                "Continuity memory remaining[{index}].{key} is not supported for pending work"
            ));
        }
        let description = fields
            .get("description")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "Continuity memory remaining[{index}].description requires a nonempty string"
                )
            })?;
        if fields.get("kind").is_some_and(|kind| {
            !matches!(
                kind.as_str(),
                Some("change" | "check" | "inspection" | "decision")
            )
        }) {
            return Err(format!(
                "Continuity memory remaining[{index}].kind requires change, check, inspection or decision"
            ));
        }
        *item = Value::string(description);
    }
    Ok(())
}
