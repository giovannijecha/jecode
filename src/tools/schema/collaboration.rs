use crate::json::Value;

pub(super) fn definition() -> Value {
    let string = |description: &str| {
        Value::object([
            ("type", Value::string("string")),
            ("description", Value::string(description)),
        ])
    };
    let task = Value::object([
        ("type", Value::string("object")),
        ("additionalProperties", Value::Bool(false)),
        (
            "required",
            Value::Array(
                ["title", "instructions", "workspace"]
                    .map(Value::string)
                    .into(),
            ),
        ),
        (
            "properties",
            Value::object([
                ("title", string("Short assignment title.")),
                (
                    "instructions",
                    string("Concrete objective, scope, expected deliverable and checks."),
                ),
                (
                    "context",
                    string(
                        "Relevant facts and source references for the collaborator's own context.",
                    ),
                ),
                (
                    "workspace",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(["shared", "isolated"].map(Value::string).into()),
                        ),
                        (
                            "description",
                            Value::string(
                                "shared uses current project files; isolated copies the project's current files, without Git metadata or root target/.tmp, into separate storage for later review and selective integration. Entries the copy cannot hold are reported as omitted_from_copy.",
                            ),
                        ),
                    ]),
                ),
            ]),
        ),
    ]);
    super::tool(
        "agent",
        "Coordinate collaborators with the same working tools and separate contexts. Only the main agent can delegate and review. After checking a returned or failed report, record the main agent's assessment with review (id, report_version, note); inspection or a text summary does not mark it reviewed. After any follow-up or integration, close the collaborator to release its resources and remove it from the active list. Returns arrive asynchronously; sent and received instructions are distinct. Calls are serialized with other tools.",
        Value::object([
            (
                "action",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(
                            [
                                "create",
                                "update",
                                "list",
                                "inspect",
                                "wait",
                                "stop",
                                "review",
                                "diff",
                                "integrate",
                                "close",
                            ]
                            .map(Value::string)
                            .into(),
                        ),
                    ),
                ]),
            ),
            (
                "tasks",
                Value::object([
                    ("type", Value::string("array")),
                    ("minItems", Value::number(1)),
                    ("maxItems", Value::number(8)),
                    ("items", task),
                    (
                        "description",
                        Value::string(
                            "Assignments for create; supply focused context rather than duplicating the entire conversation.",
                        ),
                    ),
                ]),
            ),
            (
                "id",
                string(
                    "Stable collaborator id from create/list. Required except for create/list and optional for wait.",
                ),
            ),
            (
                "instructions",
                string(
                    "Revised assignment for update; updates a running agent or starts follow-up work in its existing context.",
                ),
            ),
            ("context", string("Additional relevant context for update.")),
            ("work", super::work()),
            (
                "timeout_ms",
                Value::object([
                    ("type", Value::string("integer")),
                    ("minimum", Value::number(0)),
                    ("maximum", Value::number(60000)),
                    (
                        "description",
                        Value::string(
                            "Maximum wait time; returns sooner when a collaborator's state changes.",
                        ),
                    ),
                ]),
            ),
            (
                "report_version",
                Value::object([
                    ("type", Value::string("integer")),
                    ("minimum", Value::number(1)),
                    (
                        "description",
                        Value::string(
                            "Exact report.version of the latest returned assignment checked by the main agent; required for review.",
                        ),
                    ),
                ]),
            ),
            (
                "note",
                string(
                    "Main agent's review evidence or assessment for review. Close disposes the agent's owned isolated workspace; inspect and integrate needed changes first.",
                ),
            ),
            (
                "paths",
                Value::object([
                    ("type", Value::string("array")),
                    ("items", string("Relative changed file path.")),
                    ("minItems", Value::number(1)),
                    (
                        "description",
                        Value::string("Explicit files to integrate after inspecting diff."),
                    ),
                ]),
            ),
            (
                "expected_review_token",
                string(
                    "Review token returned by diff. Integration rechecks the main project and reports conflicts instead of overwriting concurrent changes.",
                ),
            ),
            (
                "path",
                string(
                    "For diff: one relative changed file to inspect baseline and current child content in pages.",
                ),
            ),
            (
                "offset",
                Value::object([
                    ("type", Value::string("integer")),
                    ("minimum", Value::number(0)),
                    (
                        "description",
                        Value::string(
                            "Byte offset for paged diff content; use the next offset returned by the tool.",
                        ),
                    ),
                ]),
            ),
            (
                "limit",
                Value::object([
                    ("type", Value::string("integer")),
                    ("minimum", Value::number(1)),
                    ("maximum", Value::number(65536)),
                    (
                        "description",
                        Value::string(
                            "Maximum bytes per side of paged diff content; default 16384.",
                        ),
                    ),
                ]),
            ),
        ]),
        &["action"],
    )
}
