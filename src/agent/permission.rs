//! The permission level applied to each call before it runs (B12). The
//! level decides; a request waits, unrun, for the user; the agent learns the
//! outcome as the tool result.

use super::Agent;
use crate::{
    backend::ToolCall,
    events::{Event, EventSink},
    json::Value,
    permissions::{Access, Asked, Decision, Level, Permissions, Verdict, describe},
};
use std::path::PathBuf;

pub(super) enum Gate {
    /// Run the call; `Some` is the root for an approved outside file access.
    Run {
        base: Option<PathBuf>,
        access: Option<Access>,
        level: Level,
    },
    /// The call does not run; this is its result.
    Done(Value),
}

impl Agent {
    pub fn permissions(&self) -> Permissions {
        self.permissions.clone()
    }

    /// Collaborators share the conversation's state; only the main agent
    /// restores it from the session.
    pub(super) fn bind_permissions(&self) {
        if self.child {
            self.permissions
                .bind_agent_journal(self.archive(), self.persistence.clone());
        } else {
            self.permissions
                .bind(self.archive(), self.persistence.clone());
        }
    }

    /// A call that may ask, is refused or works outside runs on its own, so
    /// parallel batches hold only calls the level allows inside the folder.
    pub(super) fn gated(&self, call: &ToolCall) -> bool {
        self.tools
            .access(&call.name, &call.arguments)
            .map_or(call.name == "exec", |access| {
                access.outside || !matches!(self.permissions.check(&access), Verdict::Allow)
            })
    }

    pub(super) fn permit(&self, call: &ToolCall, events: &mut impl EventSink) -> Gate {
        let current_level = self.permissions.level();
        if call.name == "browser"
            && current_level != Level::Full
            && !matches!(
                crate::json::parse(&call.arguments)
                    .ok()
                    .as_ref()
                    .and_then(|arguments| arguments.get("action"))
                    .and_then(Value::as_str),
                Some("close" | "stop")
            )
        {
            return Gate::Done(unavailable(
                "Browser control is available only at the Full permission level; use web for HTTP retrieval",
            ));
        }
        let Some(access) = self.tools.access(&call.name, &call.arguments) else {
            return if call.name == "exec" {
                Gate::Done(unavailable("Could not verify exec permission; nothing ran"))
            } else {
                Gate::Run {
                    base: None,
                    access: None,
                    level: current_level,
                }
            };
        };
        let (level, verdict) = self.permissions.assess(&access);
        match verdict {
            Verdict::Allow => Gate::Run {
                base: access.base.clone(),
                access: Some(access),
                level,
            },
            Verdict::Unavailable(reason) => Gate::Done(unavailable(&reason)),
            Verdict::Ask => {
                let Some(request) = self.permissions.begin(&call.id, &call.name, &access) else {
                    return Gate::Done(unavailable(&format!(
                        "Permission needed for {}, and no one can approve it here. The user can choose Full with /permissions, or run Jecode interactively.",
                        describe(&access)
                    )));
                };
                let waiting = self.permissions.pending().map_or(1, |(_, count)| count);
                let id = request.id;
                // The line-based CLI answers here; the TUI reads the queue.
                if events
                    .emit(Event::PermissionRequest {
                        request: request.clone(),
                        waiting,
                    })
                    .is_err()
                {
                    self.permissions.abandon(id);
                    return Gate::Done(Value::object([
                        (
                            "error",
                            Value::string(
                                "Tool was not executed: permission request could not be shown",
                            ),
                        ),
                        ("outcome", Value::string("not_executed")),
                    ]));
                }
                match self.permissions.wait(id, &self.cancellation) {
                    Asked::Decided(decision) => {
                        if self
                            .permissions
                            .record_agent_decision(&request, decision)
                            .is_err()
                        {
                            return Gate::Done(Value::object([
                                (
                                    "error",
                                    Value::string(
                                        "Tool was not executed: permission decision could not be saved",
                                    ),
                                ),
                                ("outcome", Value::string("not_executed")),
                            ]));
                        }
                        if decision != Decision::Deny
                            && self.tools.access(&call.name, &call.arguments).as_ref()
                                != Some(&access)
                        {
                            return Gate::Done(unavailable(
                                "The requested path or working directory changed while approval was pending; nothing ran",
                            ));
                        }
                        let note = match decision {
                            Decision::Once => "Approved once",
                            Decision::Remember => "Approved for this conversation",
                            Decision::Deny => "Denied",
                        };
                        // The row shows the decision; a failed note never
                        // changes what the user decided.
                        let _ = events.emit(Event::ToolNote {
                            id: self.redact(&call.id),
                            text: note.into(),
                        });
                        if decision == Decision::Deny {
                            Gate::Done(Value::object([
                                ("outcome", Value::string("denied")),
                                (
                                    "reason",
                                    Value::string(format!(
                                        "The user denied {}; nothing ran. Continue without it or explain what it was for.",
                                        describe(&access)
                                    )),
                                ),
                            ]))
                        } else {
                            let (level, _) = self.permissions.assess(&access);
                            if level == Level::Read {
                                Gate::Done(unavailable(
                                    "The permission level changed to Read while this call waited; nothing ran",
                                ))
                            } else {
                                Gate::Run {
                                    base: access.base.clone(),
                                    access: Some(access),
                                    level,
                                }
                            }
                        }
                    }
                    Asked::Cancelled => Gate::Done(Value::object([
                        (
                            "error",
                            Value::string(
                                "Tool was not executed: the turn stopped before a permission decision",
                            ),
                        ),
                        ("outcome", Value::string("not_executed")),
                    ])),
                }
            }
        }
    }

    pub(super) fn permission_instructions(&self) -> String {
        match self.permissions.level() {
            Level::Read => "Permissions: Read. Edits, commands, outside paths and browser control are unavailable; web GET/HEAD is available. The user can change /permissions.\n",
            Level::Workspace => "Permissions: Workspace. Bash and browser control are unavailable; use web for HTTP retrieval. On Windows, use exec powershell.exe -NoProfile -Command. Outside paths and exec cwd ask the user. A local server that needs host access requires the user to choose /permissions full first.\n",
            Level::Full => "Permissions: Full. Bash and exec may work outside the folder.\n",
        }
        .into()
    }
}

fn unavailable(reason: &str) -> Value {
    Value::object([
        ("outcome", Value::string("unavailable")),
        ("reason", Value::string(reason)),
    ])
}
