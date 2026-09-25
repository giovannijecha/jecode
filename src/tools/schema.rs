use crate::{json, providers::openai_account::Tool};

pub fn definitions() -> Vec<Tool> {
    definitions_for(&crate::command::Shell::default())
}
pub(crate) fn definitions_for(shell: &crate::command::Shell) -> Vec<Tool> {
    let mut tools: Vec<_> = [
        ("list_files", "List a local directory allowed by the session profile, sorted by name. path defaults to '.', limit defaults to 200 (1..500). Excludes dot paths, build/vendor directories, credential names, links and unsupported entries; reports omissions and truncation. Paths may be relative to the working directory or absolute when the session profile allows it.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":500}},"additionalProperties":false}"#),
        ("read_file", "Read a paginated range from an ordinary UTF-8 file, including files larger than 1 MiB. start_line is 1-based (default 1); max_lines defaults to 200 (1..400). Returns at most 8 KiB of complete lines with next_line when more remains. Very long individual lines fail. The whole file is checked during the read; cancellation or the operation time limit may stop a very large scan. Excluded paths, links and binary files cannot be read.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"max_lines":{"type":"integer","minimum":1,"maximum":400}},"required":["path"],"additionalProperties":false}"#),
        ("search_text", "Search a directory recursively for a case-sensitive literal (not a regex), one result per matching line. path defaults to '.', max_results defaults to 50 (1..100). Query is 1..256 UTF-8 bytes. Searches ordinary UTF-8 files up to 1 MiB each, bounded by 2000 files and 8 MiB of text. Results include 1-based line and byte_column; excerpts may be shortened. Check truncated and omitted before drawing conclusions.",
        r#"{"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":256},"path":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#),
        ("create_file", "Create one absent UTF-8 text file in an existing allowed local directory. Required path and content. Executes directly; the visible diff is bounded independently of the content and reports omissions. Never overwrites an existing entry or creates parent directories. No command execution.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}"#),
        ("edit_file", "Apply one exact, unique text replacement in an ordinary allowed UTF-8 file, including files larger than 1 MiB. Read relevant lines first. Required path, old_text, and new_text. Empty old_text is only valid for an empty file. Preserve existing LF/CRLF. Executes directly; the visible diff is bounded independently of the change and reports omissions. Rejects stale files. Original is retained under the reported user-scoped recovery ID.",
        r#"{"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}},"required":["path","old_text","new_text"],"additionalProperties":false}"#),
    ].into_iter().map(|(name, description, schema)| Tool {
        name: name.into(), description: description.into(),
        parameters: json::parse(schema, Default::default()).expect("owned tool schema"),
    }).collect();
    tools.push(Tool {
        name: "run_command".into(),
        description: format!("Run one non-interactive command directly using {}. command is at most 4096 UTF-8 bytes. path is the starting directory, relative to the working directory or absolute when permitted (default '.'). timeout_seconds defaults to 60 (1..300). stdin is closed. Output streams to the user; results retain up to 6 KiB per stream and report truncation. More than 1 MiB of output stops the command. No background services. The shell can access files/network beyond the workspace; it is not sandboxed. Never read or print credentials. A cancelled command may already have effects; inspect its receipt before repeating it.", shell.label()),
        parameters: json::parse(r#"{"type":"object","properties":{"command":{"type":"string","minLength":1,"maxLength":4096},"path":{"type":"string"},"timeout_seconds":{"type":"integer","minimum":1,"maximum":300}},"required":["command"],"additionalProperties":false}"#, Default::default()).expect("owned command schema"),
    });
    tools
}
