use crate::{json::Value, workspace};

pub enum Prepared {
    Image {
        path: Option<String>,
        image_id: Option<String>,
    },
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
            "view_image" => &["path", "image_id"],
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
        if name == "view_image" {
            let path = image_selector(args, "path")?;
            let image_id = image_selector(args, "image_id")?;
            if path.is_some() == image_id.is_some() {
                return Err("view_image requires exactly one of path or image_id");
            }
            if image_id.is_some_and(|id| {
                id.len() != 64
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                return Err(
                    "image_id must be a 64-character lowercase SHA-256 digest from this session",
                );
            }
            let path = path
                .map(|path| workspace::input(path).map_err(|_| "use a valid local image path"))
                .transpose()?;
            return Ok(Self::Image {
                path,
                image_id: image_id.map(str::to_owned),
            });
        }
        let path = match args.get("path") {
            Some(Value::String(path)) => path.as_str(),
            None if matches!(name, "list_files" | "search_text" | "run_command") => ".",
            _ => return Err("path must be a string; file tools require a path"),
        };
        let path = workspace::input(path).map_err(|error| match error {
            workspace::Error::Excluded => "path is excluded from workspace reads",
            _ => "use a valid local file path without hidden controls or device/network names",
        })?;
        Ok(match name {
            "run_command" => Self::Command {
                path,
                command: bounded_text(args, "command", 4096)?,
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
                start: integer(args, "start_line", 1, usize::MAX)?,
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
            Self::Image { .. } => "view_image",
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
            Self::Image {
                path: Some(path), ..
            } => path,
            Self::Image {
                image_id: Some(id), ..
            } => id,
            Self::Image { .. } => "",
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
            Self::Image { .. } => Err(workspace::ChangeError(
                "image reads cannot propose a change".into(),
                None,
            )),
            Self::Create { path, content } => workspace.prepare_create(path, content, budget),
            Self::Edit { path, old, new } => workspace.prepare_edit(path, old, new, budget),
            _ => Err(workspace::ChangeError(
                "read tool cannot propose a change".into(),
                None,
            )),
        }
    }
}
fn image_selector<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, &'static str> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => {
            Err("view_image selectors must be nonempty strings or null")
        }
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err("view_image selectors must be strings or null"),
    }
}
fn text(args: &Value, key: &str) -> Result<String, &'static str> {
    args.get(key)
        .and_then(Value::text)
        .map(String::from)
        .ok_or("text argument must be a string")
}
fn bounded_text(args: &Value, key: &str, maximum: usize) -> Result<String, &'static str> {
    args.get(key)
        .and_then(Value::text)
        .filter(|s| s.len() <= maximum)
        .map(String::from)
        .ok_or("command must be a string of at most 4096 bytes")
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
