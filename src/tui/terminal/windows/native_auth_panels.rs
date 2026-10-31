use super::{Terminal, probe};
use crate::tui::{render::Renderer, selector::Selector, state::State, view};
use std::io::{self, Write};

pub(super) fn console() {
    let terminal = Terminal::open().unwrap();
    let (width, height) = terminal.size;
    let account = crate::auth::Account {
        id: "fixture-claude".into(),
        identity: "fixture-claude".into(),
        label: "claude.account@example.test".into(),
        detail: String::new(),
        enabled: true,
    };
    let mut loading = Selector::loading(
        "Continue with Claude",
        "Waiting for Claude sign-in… · Open in browser: https://example.test/authorize",
        false,
    );
    loading.parent = Some("Anthropic");
    for (menu, expected) in [
        (Selector::credentials(true, true), "Claude plan"),
        (
            Selector::accounts("anthropic", &[account], Some("fixture-claude")),
            "claude.account@example.test",
        ),
        (loading, "Continue with Claude"),
    ] {
        let state = State {
            width,
            height,
            selector: Some(menu),
            ..State::default()
        };
        let frame = view::frame(&state, "fixture/model", "project");
        assert!(
            frame
                .live
                .iter()
                .any(|line| line.plain().contains(expected))
        );
        write!(
            io::stdout(),
            "{}",
            Renderer::new().paint(&frame, width, height)
        )
        .unwrap();
        io::stdout().flush().unwrap();
        assert!(
            probe().contains(expected),
            "native panel did not paint {expected}"
        );
    }
    drop(terminal);
}
