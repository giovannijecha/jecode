//! Model-owned recovery attempts and explicit scope acquisition.
use super::*;

pub(super) fn capture(
    workspace: &Arc<Workspace>,
    caller: &Caller<'_>,
    arguments: &Value,
) -> Result<Value, String> {
    let paths = arguments
        .get("paths")
        .and_then(Value::as_array)
        .ok_or("capture needs a list of file or directory paths")?
        .iter()
        .map(|path| {
            path.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "paths must contain strings".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Err("capture needs file or directory paths".into());
    }
    super::super::targeted::run(workspace, caller.origin, &paths)
}

pub(super) fn accept(
    workspace: &Arc<Workspace>,
    caller: &Caller<'_>,
    arguments: &Value,
) -> Result<Value, String> {
    let group = workspace.store().transaction(true, |tx| {
        let selected = string(arguments, "ref")?.map(parse_reference).transpose()?;
        let group = selected
            .map(|id| {
                tx.index()
                    .recovery
                    .snapshots
                    .get(&id)
                    .map_or(id, |snapshot| snapshot.group)
            })
            .or_else(|| super::super::recovery::group_for(tx.index(), caller.origin))
            .ok_or("There is no active recovery group for this conversation")?;
        let origin = if caller.user {
            tx.index()
                .recovery
                .groups
                .get(&group)
                .map(|group| group.origin.clone())
                .ok_or("Unknown recovery group")?
        } else {
            caller.origin.to_string()
        };
        super::super::recovery::accept(tx, group, &origin)?;
        Ok(group)
    })?;
    let store = workspace.store();
    let data = &store.index.recovery.groups[&group];
    Ok(Value::object([
        ("status", Value::string("accepted")),
        ("group", Value::string(reference(group))),
        (
            "accepted_at",
            Value::number(data.accepted.unwrap_or_default()),
        ),
        (
            "expires_at",
            Value::number(data.accepted_until.unwrap_or_default()),
        ),
        ("maximum_retention_hours", Value::number(24)),
        (
            "retained",
            Value::string("first before-state per path; user pins remain until released"),
        ),
    ]))
}
