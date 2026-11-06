use super::*;
use crate::{
    agent::Agent,
    ask::Request,
    backend::openrouter::OpenRouter,
    json,
    test_support::Directory,
    tools::Tools,
    tui::{
        Decoded, Kind,
        ask::Panel,
        selector::{Choice, Purpose},
        state::State,
        terminal::Key,
        tests, theme, view,
    },
};

const DRAFT: &str = "keep every word";

fn app(home: &Directory, project: &Directory) -> App {
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(project.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    App::new(agent, tests::config(project), None)
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    assert!(
        !app.input(Decoded::Key(Key {
            code,
            modifiers,
            character: 0,
        }))
        .unwrap()
    );
}

fn text(app: &mut App, value: &str) {
    assert!(!app.input(Decoded::Text(value.into())).unwrap());
}

const CTRL: u8 = 4;
const CTRL_G: u16 = 71;
const ENTER: u16 = 13;
const ESC: u16 = 27;

fn live(state: &State) -> Vec<String> {
    view::frame(state, "fixture/model", "fixture")
        .live
        .iter()
        .map(Line::plain)
        .collect()
}

use crate::tui::line::Line;

fn commands_open(app: &App) -> bool {
    app.state
        .selector
        .as_ref()
        .is_some_and(|menu| matches!(menu.purpose, Purpose::Commands { .. }))
}

fn drafted(home: &Directory, project: &Directory) -> App {
    let mut app = app(home, project);
    app.state.width = 80;
    app.state.height = 32;
    app.state.editor.insert(DRAFT);
    app.state.editor.cursor = 4;
    app.edited();
    app
}

#[test]
fn the_key_opens_a_sheet_over_the_draft_and_esc_returns_it_untouched() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let before = app.state.editor.clone();
    key(&mut app, CTRL_G, CTRL);
    assert!(commands_open(&app));
    let rows = live(&app.state);
    let at = |needle: &str| rows.iter().position(|row| row.contains(needle));
    let (kept, title, field) = (
        at("draft kept").unwrap(),
        at("─ Commands").unwrap(),
        at("  Filter").unwrap(),
    );
    assert!(rows[kept].starts_with(&format!("› {DRAFT}")), "{rows:#?}");
    // The kept draft, the status row, the title and the field, in that order.
    assert_eq!((title, field), (kept + 2, kept + 3), "{rows:#?}");
    assert!(rows[title].contains("21 commands"));
    // The composer is gone and the field holds the only caret.
    assert_eq!(rows.iter().filter(|row| row.starts_with("› ")).count(), 2);
    let frame = view::frame(&app.state, "fixture/model", "fixture");
    assert_eq!(frame.cursor.map(|(row, _)| row), Some(field));

    key(&mut app, ESC, 0);
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor, before);
    assert_eq!(app.state.editor.cursor, 4);
}

#[test]
fn the_list_shares_its_groups_and_order_with_the_slash_menu_until_a_filter_drops_them() {
    let menu = Selector::commands(|_| false);
    assert_eq!(menu.options.len(), COMMANDS.len());
    let slash: Vec<_> = crate::session::commands::suggestions("/")
        .iter()
        .map(|matched| COMMANDS[matched.index].name)
        .collect();
    let listed: Vec<_> = menu
        .filtered
        .iter()
        .map(|hit| menu.options[hit.index].name.as_str())
        .collect();
    assert_eq!(listed, slash);

    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    key(&mut app, CTRL_G, CTRL);
    assert!(
        live(&app.state)
            .iter()
            .any(|row| row.starts_with("  Conversation"))
    );
    text(&mut app, "mod");
    let rows = live(&app.state);
    assert!(
        rows.iter().any(|row| row.contains("1 of 21 commands")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.starts_with("  Filter  mod")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|row| row.starts_with("  Model")),
        "{rows:#?}"
    );
    assert_eq!(app.state.editor.text, DRAFT);
    text(&mut app, "zzz");
    assert!(
        live(&app.state)
            .iter()
            .any(|row| row.contains("No matching command"))
    );
    // Nothing to run, and nothing happens.
    key(&mut app, ENTER, 0);
    assert!(commands_open(&app));
}

#[test]
fn a_menu_opens_over_the_draft_esc_goes_back_one_level_and_a_choice_goes_home() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let before = app.state.editor.clone();
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "settings");
    key(&mut app, ENTER, 0);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Settings
    ));
    assert_eq!(app.state.editor, before);
    assert!(
        live(&app.state)
            .iter()
            .any(|row| row.contains("draft kept"))
    );

    // One level back, then the composer.
    key(&mut app, ESC, 0);
    assert!(commands_open(&app), "{:#?}", live(&app.state));
    key(&mut app, ESC, 0);
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor, before);

    // A direct menu: its choice goes straight to the composer, and Esc there is Esc.
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "permissions");
    key(&mut app, ENTER, 0);
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Permissions { .. }
    ));
    text(&mut app, "1");
    assert!(app.state.selector.is_none(), "{:#?}", live(&app.state));
    assert_eq!(app.state.editor, before);
    key(&mut app, ESC, 0);
    assert!(app.state.selector.is_none());
}

#[test]
fn a_settings_choice_returns_to_its_row_and_the_theme_applies_at_once() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let before = app.state.editor.clone();
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "settings");
    key(&mut app, ENTER, 0);
    // Theme is the sixth row and Light the third.
    text(&mut app, "6");
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Theme
    ));
    text(&mut app, "3");
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, Purpose::Settings));
    assert_eq!(menu.options[menu.selected].name, "Theme");
    assert_eq!(menu.options[menu.selected].fact, "Light");
    let notice = app.state.notice.as_ref().unwrap();
    assert!(matches!(notice.kind, Kind::Notice));
    assert!(
        notice
            .text
            .contains("Theme: Auto → Light · this and new conversations"),
        "{}",
        notice.text
    );
    assert_eq!(app.config.settings.theme, crate::config::Theme::Light);
    // Light was on: turning it off is a change.
    assert!(theme::set_light(false));
    assert_eq!(app.state.editor, before);
    // A chosen theme ignores the terminal's report: it stays light.
    app.terminal_background(false);
    assert!(theme::set_light(false));
    key(&mut app, ESC, 0);
    assert!(commands_open(&app));
    key(&mut app, ESC, 0);
    assert!(app.state.selector.is_none());
}

#[test]
fn a_command_without_a_sheet_leaves_no_way_back_to_the_commands() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "export");
    key(&mut app, ENTER, 0);
    assert!(app.state.selector.is_none());
    assert!(!app.from_commands);
    assert_eq!(app.state.editor.text, DRAFT);
    key(&mut app, ESC, 0);
    assert!(app.state.selector.is_none());
}

#[test]
fn new_carries_the_draft_out_of_the_conversation_it_was_written_in() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    app.capture_input();
    let old = app.persistence.as_ref().unwrap().clone();
    key(&mut app, CTRL_G, CTRL);
    text(&mut app, "new");
    key(&mut app, ENTER, 0);
    assert_ne!(app.persistence.as_ref().unwrap().id(), old.id());
    assert_eq!(app.state.editor.text, DRAFT);
    assert_eq!(app.state.editor.cursor, 4);
    assert!(!old.snapshot().input.meaningful());
    app.capture_input();
    let saved = app.persistence.as_ref().unwrap().snapshot().input;
    assert_eq!(saved.draft.text, DRAFT);
}

#[test]
fn resume_carries_the_draft_and_a_saved_draft_joins_below_it() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut earlier = app(&home, &project);
    earlier
        .agent
        .as_mut()
        .unwrap()
        .prepare_turn("Earlier work")
        .unwrap();
    let id = earlier.persistence.as_ref().unwrap().id();
    earlier.state.editor.insert("saved there");
    earlier.capture_input();
    earlier.persist_input(true);
    drop(earlier);

    let mut current = drafted(&home, &project);
    current.capture_input();
    let left = current.persistence.as_ref().unwrap().clone();
    current.resume_session(&id);
    assert_eq!(current.state.editor.text, format!("{DRAFT}\n\nsaved there"));
    assert_eq!(current.state.editor.cursor, 4);
    assert_eq!(left.snapshot().input.draft.text, "");
}

#[test]
fn resume_that_fails_hands_the_draft_back() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let before = app.state.editor.clone();
    app.resume_session("no-such-conversation");
    assert_eq!(app.state.editor, before);
}

#[test]
fn during_a_turn_what_can_run_runs_and_the_rest_says_when() {
    // What `submit` would run at once, and what it would queue.
    for name in [
        "/copy",
        "/terminal",
        "/agents",
        "/permissions",
        "/mcp",
        "/lsp",
    ] {
        assert!(!waits(name, true, true), "{name}");
        assert!(!waits(name, true, false), "{name}");
    }
    for name in [
        "/model",
        "/settings",
        "/effort",
        "/routing",
        "/help",
        "/skills",
    ] {
        assert!(!waits(name, true, true), "{name}");
        assert!(waits(name, true, false), "{name}");
    }
    for name in ["/new", "/resume", "/export", "/init", "/exit"] {
        assert!(waits(name, true, true), "{name}");
        assert!(!waits(name, false, false), "{name}");
    }

    let menu = Selector::commands(|name| waits(name, true, true));
    let state = State {
        width: 80,
        height: 40,
        selector: Some(menu),
        ..State::default()
    };
    let rows = live(&state);
    let new = rows.iter().find(|row| row.contains("/new")).unwrap();
    assert!(new.contains("after this turn"), "{rows:#?}");
    // The focus starts on the first command that can run, not on `/new`.
    let menu = state.selector.as_ref().unwrap();
    let first = menu.chosen().map(|choice| match choice {
        Choice::Command(index) => COMMANDS[index].name,
        _ => "",
    });
    assert_eq!(first, Some("/copy"));
}

#[test]
fn the_focus_steps_over_what_waits_and_stays_put_at_the_ends() {
    let mut menu = Selector::commands(|name| name != "/copy" && name != "/help");
    let name = |menu: &Selector| match menu.chosen() {
        Some(Choice::Command(index)) => COMMANDS[index].name,
        _ => "",
    };
    assert_eq!(name(&menu), "/copy");
    menu.move_selection(true);
    assert_eq!(name(&menu), "/copy");
    menu.move_selection(false);
    assert_eq!(name(&menu), "/help");
    menu.move_selection(false);
    assert_eq!(name(&menu), "/help");
    // A turn that ends frees the rows; one that starts moves the focus off them.
    assert!(menu.hold_commands(|_| false));
    assert_eq!(name(&menu), "/help");
    assert!(menu.hold_commands(|name| name == "/help"));
    assert_ne!(name(&menu), "/help");
    assert!(!menu.hold_commands(|name| name == "/help"));
    // With nothing to run, Enter has nothing to choose.
    menu.hold_commands(|_| true);
    assert!(menu.chosen().is_none());
}

#[test]
fn a_filter_with_only_waiting_commands_has_no_selected_row_or_run_key() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let draft = app.state.editor.clone();
    let id = app.persistence.as_ref().unwrap().id();
    key(&mut app, CTRL_G, CTRL);
    app.state.selector.as_mut().unwrap().hold_commands(|_| true);
    text(&mut app, "new");
    let frame = view::frame(&app.state, "fixture/model", "fixture");
    let row = frame
        .live
        .iter()
        .find(|row| row.plain().contains("/new"))
        .unwrap();
    assert!(!row.plain().contains("› /new"));
    assert!(row.plain().contains("after this turn"));
    assert!(
        row.spans
            .iter()
            .any(|span| span.text.contains("/new") && span.style == theme::DIM)
    );
    assert!(
        row.spans
            .iter()
            .all(|span| span.background != Some(theme::SELECTED_SURFACE))
    );
    assert!(
        !frame
            .live
            .iter()
            .any(|row| row.plain().contains("Enter run"))
    );
    assert!(app.state.selector.as_ref().unwrap().chosen().is_none());
    key(&mut app, ENTER, 0);
    assert!(commands_open(&app));
    assert_eq!(app.persistence.as_ref().unwrap().id(), id);
    assert_eq!(app.state.editor, draft);
    assert!(app.state.queue.is_empty());

    app.state
        .selector
        .as_mut()
        .unwrap()
        .hold_commands(|_| false);
    let frame = view::frame(&app.state, "fixture/model", "fixture");
    let row = frame
        .live
        .iter()
        .find(|row| row.plain().contains("/new"))
        .unwrap();
    assert!(row.plain().contains("› /new"));
    assert!(
        row.spans
            .iter()
            .any(|span| span.background == Some(theme::SELECTED_SURFACE))
    );
    assert!(
        frame
            .live
            .iter()
            .any(|row| row.plain().contains("Enter run"))
    );
}

#[test]
fn the_key_opens_nothing_while_ask_holds_the_keys() {
    let (home, project) = (Directory::new(), Directory::new());
    let mut app = drafted(&home, &project);
    let request = Request::parse(
        "ask",
        &json::parse(
            r#"{"questions":[{"id":"format","title":"Format","question":"Which format?",
            "options":[{"id":"csv","label":"CSV"},{"id":"json","label":"JSON"}]}]}"#,
        )
        .unwrap(),
    )
    .unwrap();
    app.state.ask_panel = Some(Panel::new(&request, None));
    app.state.question = Some(request);
    key(&mut app, CTRL_G, CTRL);
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor.text, DRAFT);
}
