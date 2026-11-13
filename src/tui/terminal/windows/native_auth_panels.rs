use super::{Terminal, probe};
use crate::tui::{
    feedback::Feedback,
    render::Renderer,
    selector::{ProviderGroup, Selector},
    state::{Kind, State},
    view,
};
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
    let endpoint = "http://127.0.0.1:1";
    let services = [
        crate::backend::Services::fixture(endpoint),
        crate::backend::Services::from_service(crate::backend::openai::Api::fixture_api_key(
            "fixture-openai-key",
            endpoint,
        )),
        crate::backend::Services::from_service(crate::backend::anthropic::Api::fixture_api_key(
            "fixture-anthropic-key",
            endpoint,
        )),
        crate::backend::Services::from_service(crate::backend::openai::Api::fixture(
            crate::auth::openai::Session::fixture("fixture-chatgpt-token"),
            endpoint,
        )),
        crate::backend::Services::from_service(crate::backend::anthropic::Api::fixture(
            crate::auth::anthropic::Session::fixture("fixture-claude-token"),
            endpoint,
        )),
    ];
    for columns in [44, width] {
        let mut loading = Selector::loading(
            "Continue with Claude",
            "Waiting for Claude sign-in… · Open in browser: https://example.test/authorize",
            false,
        );
        loading.parent = Some("Anthropic");
        for (menu, expected) in [
            (
                Selector::provider_groups(&services, false, "openrouter", "anthropic", None),
                "API keys",
            ),
            (
                Selector::provider_groups(&services, true, "openrouter", "anthropic", None),
                "Credentials",
            ),
            (
                Selector::providers(
                    &services[..3],
                    false,
                    ProviderGroup::ApiKeys,
                    "openrouter",
                    "anthropic",
                ),
                "OpenAI API",
            ),
            (
                Selector::providers(
                    &services[3..],
                    true,
                    ProviderGroup::Credentials,
                    "openrouter",
                    "anthropic",
                ),
                "Anthropic",
            ),
            (Selector::credentials(true, true), "Claude plan"),
            (
                Selector::api_keys(&[
                    ("openrouter", true),
                    ("openai-api", false),
                    ("anthropic-api", true),
                ]),
                "Anthropic API",
            ),
            (Selector::key("openai-api"), "OpenAI API key"),
            (
                Selector::accounts(
                    "anthropic",
                    std::slice::from_ref(&account),
                    Some("fixture-claude"),
                ),
                "claude.account@example.test",
            ),
            (
                Selector::account("anthropic", &account, true),
                "Manage usage",
            ),
            (loading, "Continue with Claude"),
        ] {
            let state = State {
                width: columns,
                height,
                selector: Some(menu),
                ..State::default()
            };
            paint(&state, expected, columns, height);
        }
        let state = State {
            width: columns,
            height,
            selector: Some(Selector::accounts(
                "anthropic",
                std::slice::from_ref(&account),
                None,
            )),
            notice: Some(Feedback::result(
                Kind::Warning,
                crate::tui::providers::CLAUDE_COMPATIBILITY_WARNING,
            )),
            ..State::default()
        };
        paint(&state, "Claude login unapproved", columns, height);
        // A saved key marked for removal asks before anything changes.
        let mut keys = Selector::api_keys(&[
            ("openrouter", true),
            ("openai-api", false),
            ("anthropic-api", true),
        ]);
        keys.toggle_delete();
        assert_eq!(keys.delete_target(), Some(0));
        let state = State {
            width: columns,
            height,
            selector: Some(keys),
            ..State::default()
        };
        paint(&state, "Remove this saved key?", columns, height);
        // /help lists the shell commands in its third section.
        let mut help = crate::tui::information::Information::help();
        help.section = 2;
        let state = State {
            width: columns,
            height,
            information: Some(help),
            ..State::default()
        };
        paint(&state, "jecode resume", columns, height);
    }
    drop(terminal);
}

fn paint(state: &State, expected: &str, columns: usize, height: usize) {
    let frame = view::frame(state, "fixture/model", "project");
    assert!(
        frame
            .live
            .iter()
            .any(|line| line.plain().contains(expected))
    );
    write!(
        io::stdout(),
        "{}",
        Renderer::new().paint(&frame, columns, height)
    )
    .unwrap();
    io::stdout().flush().unwrap();
    assert!(
        probe().contains(expected),
        "native panel did not paint {expected}"
    );
}
