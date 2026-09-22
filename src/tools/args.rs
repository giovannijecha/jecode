use crate::{json::Value, workspace};

pub enum Prepared {
    Command {
        command: String,
        path: String,
        timeout_seconds: u64,
    },
    Create {
        path: String,
        content: String,
    },
    Edit {
        path: String,
        old: String,
        new: String,
    },
    List {
        path: String,
        limit: usize,
    },
    Read {
        path: String,
        start: usize,
        lines: usize,
    },
    Search {
        path: String,
        query: String,
        limit: usize,
    },
}
impl Prepared {
    pub fn parse(name: &str, args: &Value) -> Result<Self, &'static str> {
        let keys: &[&str] = match name {
            "list_files" => &["path", "limit"],
            "read_file" => &["path", "start_line", "max_lines"],
            "search_text" => &["path", "query", "max_results"],
            "create_file" => &["path", "content"],
            "edit_file" => &["path", "old_text", "new_text"],
            "run_command" => &["command", "path", "timeout_seconds"],
            _ => {
                return Err("unknown tool; use only the advertised workspace tools");
            }
        };
        let Value::Object(fields) = args else {
            return Err("tool arguments must be an object");
        };
        if fields.keys().any(|key| !keys.contains(&key.as_str())) {
            return Err("unknown tool argument");
        }
        let path = match args.get("path") {
            Some(Value::String(path)) => path.as_str(),
            None if matches!(name, "list_files" | "search_text" | "run_command") => ".",
            _ => return Err("path must be a string; file tools require a path"),
        };
        let path = workspace::relative(path).map_err(|error| match error {
            workspace::Error::Excluded => "path is excluded from workspace reads",
            _ => "use a relative workspace path with forward slashes and no parent traversal",
        })?;
        Ok(match name {
            "run_command" => Self::Command {
                path,
                command: text(args, "command")?,
                timeout_seconds: integer(args, "timeout_seconds", 60, 300)? as u64,
            },
            "create_file" => Self::Create {
                path,
                content: text(args, "content")?,
            },
            "edit_file" => Self::Edit {
                path,
                old: text(args, "old_text")?,
                new: text(args, "new_text")?,
            },
            "list_files" => Self::List {
                path,
                limit: integer(args, "limit", 200, 500)?,
            },
            "read_file" => Self::Read {
                path,
                start: integer(args, "start_line", 1, 1_048_577)?,
                lines: integer(args, "max_lines", 200, 400)?,
            },
            _ => {
                let query = args.get("query").and_then(Value::text)
                    .filter(|q| !q.is_empty() && q.len() <= 256 && !q.chars().any(char::is_control))
                    .ok_or("query must be a nonempty literal, at most 256 bytes, without control characters")?;
                Self::Search {
                    path,
                    query: query.into(),
                    limit: integer(args, "max_results", 50, 100)?,
                }
            }
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::List { .. } => "list_files",
            Self::Read { .. } => "read_file",
            Self::Search { .. } => "search_text",
            Self::Create { .. } => "create_file",
            Self::Edit { .. } => "edit_file",
            Self::Command { .. } => "run_command",
        }
    }
    pub fn path(&self) -> &str {
        match self {
            Self::List { path, .. }
            | Self::Read { path, .. }
            | Self::Search { path, .. }
            | Self::Create { path, .. }
            | Self::Edit { path, .. } => path,
            Self::Command { path, .. } => path,
        }
    }
    pub fn changes_file(&self) -> bool {
        matches!(self, Self::Create { .. } | Self::Edit { .. })
    }
    pub fn propose(
        &self,
        workspace: &workspace::Workspace,
        budget: &workspace::Budget<'_>,
    ) -> Result<workspace::Change, workspace::ChangeError> {
        match self {
            Self::Create { path, content } => workspace.prepare_create(path, content, budget),
            Self::Edit { path, old, new } => workspace.prepare_edit(path, old, new, budget),
            _ => Err(workspace::ChangeError(
                "read tool cannot propose a change".into(),
            )),
        }
    }
}
fn text(args: &Value, key: &str) -> Result<String, &'static str> {
    args.get(key)
        .and_then(Value::text)
        .filter(|s| s.len() <= 32768)
        .map(String::from)
        .ok_or("text arguments must be strings of at most 32 KiB")
}
fn integer(args: &Value, key: &str, default: usize, maximum: usize) -> Result<usize, &'static str> {
    match args.get(key) {
        None => Ok(default),
        Some(value) => value
            .unsigned()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=maximum).contains(n))
            .ok_or("numeric tool argument must be an integer within its documented range"),
    }
}
