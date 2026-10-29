use super::*;
use std::path::Path;

#[test]
fn settings_restore_focus_from_their_actual_rows() {
    let path = Path::new(r"C:\fixture\custom-home\config.json");
    let menu = Selector::settings(
        "fixture/model",
        Effort::Default,
        80,
        &Routing::Auto,
        Default::default(),
        path,
    );
    let providers = menu.setting_index(Setting::Providers);
    assert_eq!(menu.options[providers].name, "Providers");
    for (index, option) in menu.options.iter().enumerate() {
        let Choice::Setting(setting) = option.choice else {
            panic!("settings menu contains a non-setting row");
        };
        assert_eq!(menu.setting_index(setting), index);
    }
}

#[test]
fn provider_settings_have_two_sections_and_account_choices_distinguish_active_from_available() {
    let providers = Selector::providers_settings();
    assert_eq!(
        providers
            .options
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["Credentials", "API keys"]
    );
    let credentials = Selector::credentials(false);
    assert_eq!(credentials.options[0].name, "OpenAI");
    assert!(credentials.options[0].badge.is_none());
    let keys = Selector::api_keys(true);
    assert_eq!(keys.options[0].name, "OpenRouter");
    assert_eq!(
        keys.options[0].badge,
        Some(Badge::Note("configured".into()))
    );

    let accounts = vec![
        crate::auth::openai::Account {
            client_id: "a".into(),
            subject: "one".into(),
            email: "one@example.test".into(),
            label: "First".into(),
            enabled: true,
        },
        crate::auth::openai::Account {
            client_id: "b".into(),
            subject: "two".into(),
            email: "two@example.test".into(),
            label: "Second".into(),
            enabled: false,
        },
    ];
    let menu = Selector::accounts(&accounts, Some("b"));
    assert_eq!(menu.options.len(), 3);
    assert_eq!(menu.options[0].badge, None);
    assert_eq!(menu.options[1].badge, Some(Badge::Current));
    assert!(matches!(menu.options[2].choice, Choice::AddAccount));
    let account = Selector::account(&accounts[0], false);
    assert!(
        account
            .options
            .iter()
            .any(|row| matches!(row.choice, Choice::SignOut))
    );
    assert!(
        account
            .options
            .iter()
            .any(|row| matches!(row.choice, Choice::ManageUsage))
    );
}

#[test]
fn provider_choice_marks_conversation_and_saved_default_separately() {
    let services = [
        crate::backend::Services::fixture("http://127.0.0.1:1"),
        crate::backend::Services::from_service(crate::backend::openai::Api::fixture(
            crate::auth::openai::Session::fixture("fixture-token"),
            "http://127.0.0.1:1",
        )),
    ];
    let menu = Selector::providers(&services, false, "openrouter", "openai");
    assert_eq!(menu.options[0].badge, Some(Badge::Current));
    assert_eq!(menu.options[1].badge, Some(Badge::Default));
    assert!(matches!(menu.options[1].choice, Choice::Provider("openai")));
}

#[test]
fn model_search_matches_friendly_names_and_keeps_selection_in_the_filtered_list() {
    let models: Vec<_> = (0..12)
        .map(|index| Model {
            id: format!("provider/model-{index}"),
            name: format!("Friendly Model {index}"),
            prompt_price: None,
            completion_price: None,
            efforts: vec![Effort::Default],
        })
        .collect();
    let mut menu = Selector::models(&models, false, "provider/model-0");
    assert!(menu.searchable);
    for _ in 0..11 {
        menu.move_selection(false);
    }
    assert!(matches!(menu.chosen(), Some(Choice::Model(11))));
    menu.editor.insert("Friendly Model 9");
    menu.refresh();
    assert_eq!(menu.filtered.len(), 1);
    assert!(!menu.filtered[0].fact.is_empty());
    assert!(matches!(menu.chosen(), Some(Choice::Model(9))));
    menu.editor.replace("no-such-model".into());
    menu.refresh();
    assert!(menu.chosen().is_none());
}

#[test]
fn search_keeps_session_metadata_and_routing_prices_out_of_rows() {
    let now = crate::sessions::now();
    let sessions: Vec<_> = (0..9)
        .map(|index| crate::sessions::Summary {
            id: format!("session-{index}"),
            title: format!("Request {index}"),
            updated: now,
            backend: "openrouter".into(),
            model: "provider/model".into(),
        })
        .collect();
    let mut menu = Selector::sessions(&sessions, "session-0");
    assert!(menu.searchable);
    assert!(menu.options.iter().all(|option| option.fact.is_empty()));
    menu.editor.insert("session-8");
    menu.refresh();
    assert_eq!(menu.filtered.len(), 1);
    assert!(matches!(menu.chosen(), Some(Choice::Session(8))));
    menu.editor.replace("provider/model".into());
    menu.refresh();
    assert_eq!(menu.filtered.len(), 9);

    let entries: Vec<_> = (0..9)
        .map(|index| Entry {
            routing: Routing::Endpoint(format!("endpoint-{index}")),
            name: format!("endpoint-{index}"),
            description: format!("$0.{index}16/$0.720 · 48 tok/s · 0.9 s"),
            metrics: Some(crate::backend::Metrics {
                input: Some(index as f64 / 10.0 + 0.016),
                output: Some(0.72),
                throughput: Some(48.0),
                latency: Some(0.9),
            }),
            facts: "fp8".into(),
            help: "Pinned endpoint".into(),
            problem: None,
        })
        .collect();
    let current = Routing::Auto;
    let mut menu = Selector::routing("provider/model", false, entries, &current, vec![]);
    assert!(menu.searchable);
    assert!(menu.options.iter().all(|option| option.fact == "fp8"));
    menu.editor.insert("$0.816");
    menu.refresh();
    assert_eq!(menu.filtered.len(), 1);
    assert!(matches!(menu.chosen(), Some(Choice::Routing(8))));
}

#[test]
fn draft_search_keeps_queued_and_paused_statuses() {
    let mut queue = crate::tui::drafts::Queue::default();
    for index in 0..8 {
        queue.push(format!("Draft {index}")).unwrap();
    }
    queue.pause();
    queue.push("Later draft").unwrap();
    let mut menu = Selector::drafts(&queue);
    assert!(menu.searchable);
    menu.editor.insert("Queued");
    menu.refresh();
    assert_eq!(menu.filtered.len(), 1);
    menu.editor.replace("Paused".into());
    menu.refresh();
    assert_eq!(menu.filtered.len(), 8);
}

#[test]
fn model_rows_drop_a_listing_name_that_only_restates_the_id() {
    assert!(restates(
        "anthropic/claude-opus-5.5",
        "Anthropic: Claude Opus 5.5"
    ));
    assert!(restates("openai/gpt-6", "OpenAI: GPT-6"));
    assert!(restates("deepseek/deepseek-v4", "DeepSeek: V4"));
    assert!(!restates(
        "openai/gpt-6.1-sol",
        "GPT-6.1 Sol (ChatGPT plan)"
    ));
    assert!(!restates("openrouter/auto", "Auto Router"));
}
