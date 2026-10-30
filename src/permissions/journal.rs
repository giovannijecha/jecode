//! Session events for permissions: the level, remembered grants and the
//! requests of the main agent with their decisions.

use super::{Decision, Grant, Kind, Level, Request};
use crate::json::Value;

pub(super) fn level(level: Level) -> Value {
    Value::object([
        ("type", Value::string("permission_level")),
        ("level", Value::string(level.id())),
    ])
}

pub(super) fn grant(kind: &str, grant: &Grant) -> Value {
    with_identity(
        with_place(
            Value::object([
                ("type", Value::string(kind)),
                ("access", Value::string(grant.kind.id())),
                ("target", Value::string(&grant.target)),
            ]),
            grant.place.as_deref(),
        ),
        grant.identity.as_deref(),
    )
}

pub(super) fn request(request: &Request) -> Value {
    with_place(
        Value::object([
            ("type", Value::string("permission_request")),
            ("tool_call_id", Value::string(&request.call_id)),
            ("tool", Value::string(&request.tool)),
            ("access", Value::string(request.access.kind.id())),
            ("target", Value::string(&request.access.target)),
        ]),
        request.access.place.as_deref(),
    )
}

fn with_place(mut event: Value, place: Option<&str>) -> Value {
    if let (Value::Object(fields), Some(place)) = (&mut event, place) {
        fields.insert("place".into(), Value::string(place));
    }
    event
}

fn with_identity(mut event: Value, identity: Option<&str>) -> Value {
    if let (Value::Object(fields), Some(identity)) = (&mut event, identity) {
        fields.insert("identity".into(), Value::string(identity));
    }
    event
}

pub(super) fn decision(request: &Request, decision: Decision) -> Value {
    let event = Value::object([
        ("type", Value::string("permission_decision")),
        ("tool_call_id", Value::string(&request.call_id)),
        ("decision", Value::string(decision.id())),
    ]);
    if decision != Decision::Remember {
        return event;
    }
    let mut event = with_identity(
        with_place(event, request.access.place.as_deref()),
        request.access.identity.as_deref(),
    );
    if let Value::Object(fields) = &mut event {
        fields.insert("access".into(), Value::string(request.access.kind.id()));
        fields.insert("target".into(), Value::string(&request.access.target));
    }
    event
}

fn saved_grant(event: &Value) -> Option<Grant> {
    Some(Grant {
        kind: Kind::parse(event.get("access")?.as_str()?)?,
        target: event.get("target")?.as_str()?.to_owned(),
        place: event
            .get("place")
            .and_then(Value::as_str)
            .map(str::to_owned),
        identity: event
            .get("identity")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// The last saved level, if any, and the grants still remembered.
pub(crate) fn replay(events: &[Value]) -> (Option<Level>, Vec<Grant>) {
    let mut level = None;
    let mut grants: Vec<Grant> = Vec::new();
    for event in events {
        match event.get("type").and_then(Value::as_str) {
            Some("permission_level") => {
                if let Some(saved) = event
                    .get("level")
                    .and_then(Value::as_str)
                    .and_then(Level::parse)
                {
                    level = Some(saved);
                }
            }
            Some("permission_grant") => {
                if let Some(grant) = saved_grant(event)
                    && !grants.contains(&grant)
                {
                    grants.push(grant);
                }
            }
            Some("permission_decision")
                if event.get("decision").and_then(Value::as_str) == Some("remember") =>
            {
                if let Some(grant) = saved_grant(event)
                    && !grants.contains(&grant)
                {
                    grants.push(grant);
                }
            }
            Some("permission_revoke") => {
                if let Some(grant) = saved_grant(event) {
                    grants.retain(|known| *known != grant);
                }
            }
            _ => {}
        }
    }
    (level, grants)
}

/// Whether the call was waiting for a permission decision when Jecode quit.
pub(crate) fn pending_request(events: &[Value], call_id: &str) -> bool {
    let mut waiting = false;
    for event in events {
        if event.get("tool_call_id").and_then(Value::as_str) != Some(call_id) {
            continue;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("permission_request") => waiting = true,
            Some("permission_decision") => waiting = false,
            _ => {}
        }
    }
    waiting
}
