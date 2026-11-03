use super::Outcome;
use crate::json::Value;

impl Outcome {
    pub(super) fn basic(status: &str, detail: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            server: None,
            root: None,
            locations: vec![],
            diagnostics: vec![],
            freshness: "unavailable".into(),
            complete: true,
            truncated: false,
            detail: detail.into(),
            servers: vec![],
            document_version: None,
            instance_generation: None,
        }
    }
    pub fn to_value(&self) -> Value {
        Value::object([
            ("status", Value::string(&self.status)),
            (
                "server",
                self.server.as_ref().map_or(Value::Null, Value::string),
            ),
            (
                "root",
                self.root
                    .as_ref()
                    .map_or(Value::Null, |p| Value::string(p.to_string_lossy())),
            ),
            ("locations", Value::Array(self.locations.clone())),
            ("diagnostics", Value::Array(self.diagnostics.clone())),
            ("freshness", Value::string(&self.freshness)),
            ("complete", Value::Bool(self.complete)),
            ("truncated", Value::Bool(self.truncated)),
            ("detail", Value::string(&self.detail)),
            (
                "document_version",
                self.document_version.map_or(Value::Null, Value::number),
            ),
            (
                "instance_generation",
                self.instance_generation.map_or(Value::Null, Value::number),
            ),
            (
                "servers",
                Value::Array(
                    self.servers
                        .iter()
                        .map(|s| {
                            Value::object([
                                ("id", Value::string(&s.server)),
                                (
                                    "root",
                                    s.root.as_ref().map_or(Value::Null, |p| {
                                        Value::string(p.to_string_lossy())
                                    }),
                                ),
                                ("state", Value::string(&s.state)),
                                ("detail", Value::string(&s.detail)),
                                (
                                    "capabilities",
                                    s.capabilities.clone().unwrap_or(Value::Null),
                                ),
                                ("pid", s.pid.map_or(Value::Null, Value::number)),
                                (
                                    "generation",
                                    s.generation.map_or(Value::Null, Value::number),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}
