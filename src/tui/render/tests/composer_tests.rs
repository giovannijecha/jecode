use super::*;
use crate::tui::{activity::Activity, selector::Selector};

#[test]
fn composer_navigation_updates_one_border_and_erases_the_range_when_all_text_fits() {
    let mut state = State::default();
    state.editor.insert(
        "draft one\ndraft two\ndraft three\ndraft four\ndraft five\ndraft six\ndraft seven",
    );
    let mut screen = Screen::new(state.width, state.height);
    let mut renderer = Renderer::new();
    update(&mut renderer, &mut screen, &mut state);
    assert!(screen.visible().contains("3–7 / 7"));
    assert!(!screen.visible().contains("draft one"));
    assert!(!screen.visible().contains("more lines"));
    state.editor.cursor = "draft one\ndraft two\ndraft three\ndra".len();
    update(&mut renderer, &mut screen, &mut state);
    assert!(screen.visible().contains("2–6 / 7"));
    assert!(!screen.visible().contains("3–7 / 7"));
    assert!(!screen.visible().contains("draft seven"));
    state.editor.replace("short draft\nsecond line".into());
    update(&mut renderer, &mut screen, &mut state);
    assert!(screen.visible().contains("short draft"));
    assert!(!screen.visible().contains(" / 7"));
    assert!(!screen.visible().contains("draft five"));
    assert!(screen.history.is_empty());
}

#[test]
fn command_and_menu_panels_are_erased_on_close_and_never_enter_native_history() {
    let mut state = State {
        height: 10,
        ..State::default()
    };
    state.message(Kind::Assistant, &"retained answer\n".repeat(20));
    let mut screen = Screen::new(state.width, state.height);
    let mut renderer = Renderer::new();
    update(&mut renderer, &mut screen, &mut state);
    for command in ["/", "/mo", "/tmp clean"] {
        state.editor.replace(command.into());
        state.suggestions.refresh(command);
        let opened = update(&mut renderer, &mut screen, &mut state);
        assert!(opened.contains(crate::tui::theme::USER_BACKGROUND));
        assert!(screen.visible().contains("Commands"));
        state.selector = Some(Selector::settings(
            "fixture/model",
            crate::effort::Effort::High,
        ));
        update(&mut renderer, &mut screen, &mut state);
        assert!(screen.visible().contains("Settings · saved defaults"));
        assert!(!screen.visible().contains("Commands"));
        state.selector = None;
        state.suggestions.dismiss();
        update(&mut renderer, &mut screen, &mut state);
        assert!(!screen.visible().contains("Settings · saved defaults"));
        assert!(!screen.visible().contains("Tab complete"));
        assert!(screen.visible().contains(command));
    }
    state.editor.take();
    state.suggestions.refresh("");
    state.message(Kind::Assistant, "Next retained answer.");
    update(&mut renderer, &mut screen, &mut state);
    let transcript = screen.history.join("\n") + "\n" + &screen.visible();
    assert!(transcript.contains("Next retained answer."));
    assert_eq!(
        state
            .rows()
            .iter()
            .flatten()
            .filter(|line| line.plain() == "retained answer")
            .count(),
        20
    );
    assert!(screen.history.iter().all(|line| {
        !line.contains("Commands")
            && !line.contains("saved defaults")
            && !line.contains("/tmp clean")
    }));
}

#[test]
fn streamed_growth_animations_queued_rows_and_selectors_never_pollute_native_history() {
    let mut state = State {
        height: 12,
        ..State::default()
    };
    state.message(Kind::Assistant, &"retained paragraph\n".repeat(40));
    let mut screen = Screen::new(state.width, state.height);
    let mut renderer = Renderer::new();
    update(&mut renderer, &mut screen, &mut state);
    state.activity = Some(Activity::new());
    state.queue.push("queued pending".into()).unwrap();
    let mut streamed = String::new();
    for _ in 0..20 {
        streamed.push_str("streamed paragraph\n");
        state.event(Event::Streaming {
            text: streamed.clone(),
        });
        update(&mut renderer, &mut screen, &mut state);
    }
    assert!(
        screen
            .history
            .iter()
            .all(|line| !line.contains("queued pending")
                && !line.contains("Working")
                && !line.contains("Ask anything"))
    );
    state.event(Event::Message { text: streamed });
    state.activity = None;
    state.queue.messages.clear();
    update(&mut renderer, &mut screen, &mut state);
    state
        .editor
        .insert("one\ntwo\nthree\nfour\nfive\nsix\nseven");
    state.selector = Some(Selector::settings(
        "fixture/model",
        crate::effort::Effort::Default,
    ));
    let opened = update(&mut renderer, &mut screen, &mut state);
    assert!(!opened.contains("streamed paragraph"));
    state.selector = None;
    update(&mut renderer, &mut screen, &mut state);
    let transcript = screen.history.join("\n") + "\n" + &screen.visible();
    assert!(transcript.contains("streamed paragraph"));
    assert_eq!(
        state
            .rows()
            .iter()
            .flatten()
            .filter(|line| line.plain() == "retained paragraph")
            .count(),
        40
    );
    assert_eq!(
        state
            .rows()
            .iter()
            .flatten()
            .filter(|line| line.plain() == "streamed paragraph")
            .count(),
        20
    );
    assert!(
        screen
            .history
            .iter()
            .all(|line| !line.contains("saved defaults")
                && !line.contains("more lines")
                && !line.contains("queued pending"))
    );
    assert_eq!(state.editor.text, "one\ntwo\nthree\nfour\nfive\nsix\nseven");
}
