//! Field labels, placeholders and masking for the shared selector sheet.

use super::{super::panel::Field, noun};
use crate::tui::{
    lsp::{Field as LspField, Panel as LspPanel},
    mcp::{Field as McpField, Panel as McpPanel},
    selector::{Purpose, Selector},
};

/// The value typed for this menu. The label names the field and the title the panel, so
/// a placeholder only adds what neither says.
pub(super) fn render(menu: &Selector) -> Option<Field> {
    let (label, placeholder, suffix) = match &menu.purpose {
        Purpose::Key(_) => ("Paste your key".to_string(), "", ""),
        Purpose::AutoCompact => ("Threshold".into(), "1–99", "% of the context window"),
        Purpose::Mcp(McpPanel::Field(field)) => match field {
            McpField::Name => ("Shown in /mcp and tool cards".into(), "", ""),
            McpField::Endpoint => ("Server address".into(), "https://…", ""),
            McpField::Token => ("Paste your token".into(), "", ""),
            McpField::Timeout => ("Per call".into(), "1–600", " seconds"),
            McpField::Header => ("Name: value · Name: removes".into(), "Name: value", ""),
            McpField::ClientId => ("Public OAuth client ID".into(), "", ""),
        },
        Purpose::Lsp(LspPanel::Field(field)) => match field {
            LspField::Name => ("Shown in /lsp and tool cards".into(), "", ""),
            LspField::Executable => ("User-selected executable or absolute path".into(), "", ""),
            LspField::Languages => (
                "Language ID and file extensions".into(),
                "rust: .rs; cpp: .cc .cpp",
                "",
            ),
            LspField::Arguments => (
                "Quoted arguments keep spaces".into(),
                "--flag \"two words\"",
                "",
            ),
            LspField::Environment => (
                "NAME=value · NAME= removes · values hidden".into(),
                "NAME=value",
                "",
            ),
            LspField::RootMarkers => (
                "Comma-separated files inside the workspace".into(),
                "Cargo.toml",
                "",
            ),
            LspField::RootOverride => ("Explicit analysis root".into(), "Automatic", ""),
            LspField::Initialization | LspField::Settings => ("JSON value".into(), "{}", ""),
            LspField::Timeout => ("Per request".into(), "100–120000", " ms"),
        },
        Purpose::Commands { .. } | Purpose::Skills { .. } | Purpose::Files(_) => {
            ("Filter".into(), "", "")
        }
        _ if menu.searchable => (format!("Filter {}", noun(menu)), "", ""),
        _ => return None,
    };
    // The whole value: the sheet wraps it and keeps the caret in view.
    let (value, caret) = menu.editor.viewport(
        usize::MAX,
        matches!(
            menu.purpose,
            Purpose::Key(_)
                | Purpose::Mcp(McpPanel::Field(McpField::Token | McpField::Header))
                | Purpose::Lsp(LspPanel::Field(LspField::Environment))
        ),
    );
    Some(Field {
        label,
        value,
        caret,
        placeholder,
        suffix,
    })
}
