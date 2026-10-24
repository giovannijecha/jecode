use super::*;

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
