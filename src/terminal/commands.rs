//! Local commands do not expand the model's tool schema.
use super::{
    Key,
    model::{Block, Model},
};
use crate::session::Session;
const COMMANDS: &[&str] = &["/compact", "/context", "/help", "/quit"];

pub(super) fn input(model: &mut Model, key: &Key, session: &mut Session) -> bool {
    if !model.editor.text.starts_with('/') {
        return false;
    }
    if *key == Key::Tab {
        let matches: Vec<_> = COMMANDS
            .iter()
            .filter(|name| name.starts_with(&model.editor.text))
            .collect();
        if matches.len() == 1 {
            model.editor.text = (*matches[0]).into();
            model.editor.cursor = model.editor.text.len();
        }
        return true;
    }
    if *key != Key::Enter {
        return false;
    }
    let command = model.editor.text.trim();
    let handled = match command {
        "/context" => session.inspect_context(),
        "/compact" => session.compact(),
        "/help" => {
            model.blocks.push(Block { speaker: "Status", text: "/context — inspect measured context and token usage\n/compact — summarize earlier context, retaining canonical history\n/quit — save and exit\nTab completes local commands. Use --sessions and --resume ID to reopen a conversation.".into() });
            true
        }
        "/quit" => {
            model.quit = true;
            true
        }
        _ => {
            if let Some(view) = &mut model.account {
                view.notice = "Unknown local command / Tab completes; /help lists commands".into();
            }
            return true;
        }
    };
    if handled {
        let compact = command == "/compact";
        model.editor.take();
        if compact {
            super::account::compacting(model);
        }
    } else if let Some(view) = &mut model.account {
        view.notice = "Wait for this turn to finish / draft kept".into();
    }
    true
}

pub(super) fn hint(model: &Model) -> Option<String> {
    (model.account.is_some() && model.editor.text.starts_with('/')).then(|| {
        let names: Vec<_> = COMMANDS
            .iter()
            .copied()
            .filter(|name| name.starts_with(&model.editor.text))
            .collect();
        if names.is_empty() {
            "Unknown command / use /help".into()
        } else {
            format!("{}  ·  Tab completes", names.join("  "))
        }
    })
}
