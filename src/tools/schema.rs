use crate::{json, providers::openai_account::Tool};

pub(crate) const COMMAND_REACH: &str = "The shell is not sandboxed and may access files outside the workspace. It can attempt network operations through available programs; connectivity, installed programs, browsers and remote services are unverified until results establish them.";

pub fn definitions() -> Vec<Tool> {
    definitions_for(&crate::command::Shell::default(), false)
}
pub(crate) fn definitions_for(shell: &crate::command::Shell, image: bool) -> Vec<Tool> {
    let native_completion_note = if cfg!(windows) {
        " On Windows PowerShell, a GUI executable invoked with & may return before it finishes; $LASTEXITCODE and captured output can be stale. For verification, wait explicitly, capture stdout and stderr separately, and exit nonzero if required content or cleanup fails."
    } else {
        ""
    };
    let mut tools: Vec<_> = [
        ("list_files", "List a local directory allowed by the session profile, sorted by name. path defaults to '.', limit defaults to 200 (1..500). Excludes dot paths, build/vendor directories, credential names, links and unsupported entries; reports omissions and truncation. Paths may be relative to the working directory or absolute when the session profile allows it.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":500}},"additionalProperties":false}"#),
        ("read_file", "Read a paginated range from an ordinary UTF-8 file, including files larger than 1 MiB. start_line is 1-based (default 1); max_lines defaults to 200 (1..400). Returns at most 8 KiB of complete lines with next_line when more remains. Very long individual lines fail. The whole file is checked during the read; cancellation or the operation time limit may stop a very large scan. Excluded paths, links and binary files cannot be read.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"max_lines":{"type":"integer","minimum":1,"maximum":400}},"required":["path"],"additionalProperties":false}"#),
        ("recall_receipts", "Read exact saved workspace-read receipts without rerunning tools. To locate receipts covered by compaction independently of its summary, use {mode:index,turn:0,step:0,receipt:0}; follow the index next cursor for ordered, bounded pages of original call names, arguments and guarded recall_address objects. Index entries are addresses, not source contents. To retrieve content, pass a recall_address unchanged, including expected_call_id; follow its next cursor for remaining UTF-8 bytes. Response output_index and compaction record/fragment numbers are not receipt indices. Only recorded list_files, read_file and search_text results are available. If a batch says not admitted, consume delivered pages and request missing pages later. These are historical observations; use a new read separately if current source state matters.",
        r#"{"type":"object","properties":{"mode":{"type":"string","enum":["index"]},"turn":{"type":"integer","minimum":0},"step":{"type":"integer","minimum":0},"receipt":{"type":"integer","minimum":0},"offset":{"type":"integer","minimum":0},"expected_call_id":{"type":"string","minLength":1,"maxLength":256}},"required":["turn","step"],"additionalProperties":false}"#),
        ("search_text", "Search a directory recursively for a case-sensitive literal (not a regex), one result per matching line. path defaults to '.', max_results defaults to 50 (1..100). Query is 1..256 UTF-8 bytes. Searches ordinary UTF-8 files up to 1 MiB each, bounded by 2000 files and 8 MiB of text. Results include 1-based line and byte_column; excerpts may be shortened. Check truncated and omitted before drawing conclusions.",
        r#"{"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":256},"path":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#),
        ("create_file", "Create one absent UTF-8 text file in an existing allowed local directory. Required path and content. Executes directly; the visible diff is bounded independently of the content and reports omissions. Never overwrites an existing entry or creates parent directories. No command execution.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}"#),
        ("edit_file", "Apply one exact, unique text replacement in an ordinary allowed UTF-8 file, including files larger than 1 MiB. Read relevant lines first. Required path, old_text, and new_text. Empty old_text is only valid for an empty file. Preserve existing LF/CRLF. Executes directly; the visible diff is bounded independently of the change and reports omissions. Rejects stale files. Original is retained under the reported user-scoped recovery ID.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}},"required":["path","old_text","new_text"],"additionalProperties":false}"#),
    ].into_iter().map(|(name, description, schema)| Tool {
        name: name.into(), description: description.into(),
        parameters: json::parse(schema, Default::default()).expect("owned tool schema"),
        strict: None,
    }).collect();
    tools.push(Tool {
        name: "run_command".into(),
        description: format!("Run one non-interactive command directly using {}. command is at most 4096 UTF-8 bytes. path is the starting directory, relative to the working directory or absolute when permitted (default '.'). timeout_seconds defaults to 60 (1..300). stdin is closed. Output streams to the user; results retain up to 6 KiB per stream and report truncation. More than 1 MiB of output stops the command. No background services. {COMMAND_REACH} Never read or print credentials. A cancelled command may already have effects; inspect its receipt before repeating it.{native_completion_note}", shell.label()),
        parameters: json::parse(r#"{"type":"object","properties":{"command":{"type":"string","minLength":1,"maxLength":4096},"path":{"type":"string"},"timeout_seconds":{"type":"integer","minimum":1,"maximum":300}},"required":["command"],"additionalProperties":false}"#, Default::default()).expect("owned command schema"),
        strict: None,
    });
    if image {
        tools.push(Tool {
            name: "view_image".into(),
            description: "View a local PNG image as actual visual input. Set exactly one selector to a valid nonempty value and the other to null: path for a local file, or image_id for a 64-character lowercase SHA-256 ID from this session's earlier view. A path may be relative to the selected working directory or absolute when its file-access profile allows it. PNG bytes are captured privately before success; unsupported, invalid, missing and oversized files fail explicitly. Screenshot creation is outside this tool. Original pixels are sent unchanged with high detail; provider-side processing is not locally observable.".into(),
            parameters: json::parse(r#"{"type":"object","properties":{"path":{"type":["string","null"]},"image_id":{"type":["string","null"]}},"required":["path","image_id"],"additionalProperties":false}"#, Default::default()).expect("owned image tool schema"),
            strict: Some(true),
        });
    }
    tools
}
