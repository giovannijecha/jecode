use super::super::{
    App, Decoded, SessionConfig, selector, terminal,
    tests::{config, finish, wait_until},
    view,
};
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    json::Value,
    permissions::{Access, Kind, Level},
    test_support::{Directory, HttpFixture, completion, tool_call},
    tools::Tools,
};
use std::fs;

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let agent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(directory.path()).unwrap(),
    );
    let SessionConfig { store, settings } = config(directory);
    store.save(&settings).unwrap();
    App::new(agent, SessionConfig { store, settings }, None)
}

fn key(app: &mut App, code: u16, modifiers: u8) {
    app.input(Decoded::Key(terminal::Key {
        code,
        modifiers,
        character: 0,
    }))
    .unwrap();
}

fn text(app: &mut App, value: &str) {
    app.input(Decoded::Text(value.into())).unwrap();
}

fn screen(app: &App) -> String {
    view::frame(&app.state, "fixture/model", "directory")
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

fn notice(app: &App) -> String {
    app.state.notice.as_ref().unwrap().text.clone()
}

fn outside(kind: Kind, target: &str) -> Access {
    Access {
        kind,
        target: target.into(),
        place: None,
        outside: true,
        base: None,
        identity: None,
        members: Vec::new(),
    }
}

/// The sizes a request meets, from a wide window to a short, narrow one.
const SIZES: [(usize, usize); 5] = [(80, 24), (100, 40), (44, 24), (80, 12), (40, 16)];

/// Every word of `seen` that PgDn reveals from the top of the call, checking
/// at each stop that the choices and the paging key stayed on screen.
fn page_through(app: &mut App, width: usize, height: usize, words: &[String]) -> Vec<String> {
    app.state.width = width;
    app.state.height = height;
    for _ in 0..words.len() {
        key(app, 33, 0);
    }
    let mut seen = Vec::new();
    for _ in 0..words.len() {
        let shown = screen(app);
        for choice in [
            "Approve",
            "Approve and remember",
            "Deny",
            "1–3",
            "PgUp/PgDn",
        ] {
            assert!(
                shown.contains(choice),
                "{width}x{height} lost {choice}\n{shown}"
            );
        }
        seen.extend(words.iter().filter(|word| shown.contains(*word)).cloned());
        key(app, 34, 0);
    }
    seen.sort();
    seen.dedup();
    seen
}

#[test]
fn a_long_command_keeps_its_choices_and_pages_through_every_line() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.permission_observer
        .set_surface(crate::permissions::Surface::Panel);
    let lines: Vec<String> = (0..60).map(|line| format!("line-{line:02}")).collect();
    let command = lines
        .iter()
        .map(|line| format!("echo {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.permission_observer
        .begin("call", "bash", &outside(Kind::Runs, &command))
        .unwrap();
    app.refresh_permission();
    for (width, height) in SIZES {
        assert_eq!(
            page_through(&mut app, width, height, &lines),
            lines,
            "{width}x{height}"
        );
        // The last stop still names what the keys can bring back.
        assert!(screen(&app).contains(" lines"), "{}", screen(&app));
        // The tool name stays while the call scrolls.
        assert!(screen(&app).contains("BASH"), "{}", screen(&app));
    }
}

#[test]
fn a_call_that_fits_leaves_page_keys_to_the_conversation() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.permission_observer
        .set_surface(crate::permissions::Surface::Panel);
    app.permission_observer
        .begin("call", "bash", &outside(Kind::Runs, "cargo test session"))
        .unwrap();
    app.refresh_permission();
    let before = screen(&app);
    assert!(!before.contains("PgUp/PgDn"), "{before}");
    key(&mut app, 34, 0);
    assert_eq!(screen(&app), before);
}

#[test]
fn a_multi_file_request_keeps_its_choices_and_pages_through_every_file() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.permission_observer
        .set_surface(crate::permissions::Surface::Panel);
    let request = |count: usize| {
        let members: Vec<Access> = (0..count)
            .map(|index| {
                outside(
                    Kind::Edits,
                    &format!(r"D:\work\a-long-folder-name\another-long-folder\file-{index:02}.rs"),
                )
            })
            .collect();
        Access {
            target: members
                .iter()
                .map(|member| member.target.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            members,
            ..outside(Kind::Edits, "")
        }
    };
    let names: Vec<String> = (0..12).map(|index| format!("file-{index:02}.rs")).collect();
    let asked = app
        .permission_observer
        .begin("call", "edit", &request(12))
        .unwrap();
    app.refresh_permission();
    for (width, height) in [(80, 24), (44, 24), (80, 12)] {
        assert_eq!(
            page_through(&mut app, width, height, &names),
            names,
            "{width}x{height}"
        );
    }
    app.permission_observer
        .decide(asked.id, crate::permissions::Decision::Deny)
        .unwrap();
    app.refresh_permission();
    // With room, every file is named and nothing pages.
    app.permission_observer
        .begin("call", "edit", &request(2))
        .unwrap();
    app.refresh_permission();
    app.state.width = 100;
    app.state.height = 40;
    let shown = screen(&app);
    assert!(
        shown.contains("file-00.rs") && shown.contains("file-01.rs"),
        "{shown}"
    );
    assert!(!shown.contains("PgUp/PgDn"), "{shown}");
    assert!(
        shown.contains("Edits to these files, this conversation"),
        "{shown}"
    );
}

#[test]
fn the_level_menu_applies_at_once_and_the_composer_names_the_level() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    assert!(screen(&app).contains(" Workspace  ─"));
    app.dispatch("/permissions".into()).unwrap();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        menu.purpose,
        selector::Purpose::Permissions {
            default: Level::Workspace
        }
    ));
    assert_eq!(menu.selected, 1, "the focus opens on the current level");
    let shown = screen(&app);
    assert!(shown.contains("Edit and run here and in subfolders; network on"));
    assert!(shown.contains("Remembered") && shown.contains("none"));
    assert!(shown.contains("new conversations use Workspace"));
    // The current level closes with no notice and no change.
    text(&mut app, "2");
    assert!(app.state.selector.is_none() && app.state.notice.is_none());
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "1");
    assert_eq!(app.permission_observer.level(), Level::Read);
    assert!(notice(&app).contains("Permissions: Workspace → Read · this conversation"));
    assert!(screen(&app).contains(" Read  ─"));
    // Remembered opens empty and Esc returns to its row.
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "4");
    assert!(screen(&app).contains("Nothing remembered"));
    key(&mut app, 27, 0);
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        menu.purpose,
        selector::Purpose::Permissions { .. }
    ));
    assert_eq!(menu.selected, 3);
    key(&mut app, 27, 0);
    assert!(app.state.selector.is_none());
    app.dispatch("/permissions full".into()).unwrap();
    assert_eq!(app.permission_observer.level(), Level::Full);
}

#[test]
fn the_saved_default_leaves_this_conversation_level_alone() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.permission_observer.set_level(Level::Full).unwrap();
    app.dispatch("/settings".into()).unwrap();
    assert!(screen(&app).contains("Default permissions"));
    text(&mut app, "4");
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(
        menu.purpose,
        selector::Purpose::DefaultPermissions
    ));
    text(&mut app, "1");
    assert_eq!(
        app.config
            .store
            .load()
            .unwrap()
            .unwrap()
            .default_permissions,
        Level::Read
    );
    assert_eq!(app.permission_observer.default_level(), Level::Read);
    assert_eq!(app.permission_observer.level(), Level::Full);
    assert!(
        notice(&app)
            .contains("Default permissions: Workspace → Read · this conversation stays Full")
    );
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, selector::Purpose::Settings));
    assert_eq!(
        menu.selected,
        menu.setting_index(selector::Setting::Permissions)
    );
}

#[test]
fn a_request_holds_the_keys_and_a_remembered_grant_can_be_revoked() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("kept draft");
    let collaborator = app.permission_observer.for_agent("a1");
    let read = outside(Kind::Reads, r"C:\elsewhere\session.rs");
    let first = collaborator.begin("child-call", "read", &read).unwrap();
    app.permission_observer
        .begin("main-call", "bash", &outside(Kind::Runs, "cargo test"))
        .unwrap();
    app.poll().unwrap();
    let shown = screen(&app);
    for expected in [
        "Waiting for permission",
        "Permission",
        "AGENT 1",
        "· 1 of 2",
        "READ",
        "Read access outside this folder",
        "Only this read",
        "Reads of this file, this conversation",
        "Nothing runs; the agent is told",
        "1–3 choose",
    ] {
        assert!(shown.contains(expected), "{expected} in\n{shown}");
    }
    // The draft waits on one row above the request, never in a composer.
    assert_eq!(shown.matches("kept draft").count(), 1, "{shown}");
    assert!(
        shown
            .lines()
            .any(|row| row.starts_with("› kept draft") && row.ends_with("draft kept")),
        "{shown}"
    );
    // Letters and Enter alone decide nothing; ↓ then Enter does.
    text(&mut app, "y");
    key(&mut app, 13, 0);
    assert_eq!(app.permission_observer.pending().unwrap().0, first);
    assert_eq!(app.state.editor.text, "kept draft");
    key(&mut app, 40, 0);
    key(&mut app, 40, 0);
    assert!(screen(&app).contains("Enter choose"));
    key(&mut app, 13, 0);
    assert_eq!(app.permission_observer.grants(), vec![read.grant()]);
    assert!(notice(&app).contains("Permission: Reads of this file · remembered"));
    // The main agent's request follows, with the asking agent's badge.
    let shown = screen(&app);
    assert!(shown.contains("MAIN AGENT") && !shown.contains("1 of"));
    text(&mut app, "3");
    assert!(app.state.permission.is_none());
    assert!(screen(&app).contains("kept draft"));

    app.dispatch("/permissions".into()).unwrap();
    assert!(screen(&app).contains("1 for this conversation"));
    text(&mut app, "4");
    assert!(screen(&app).contains("Reads") && screen(&app).contains("session.rs"));
    key(&mut app, 68, 4);
    assert!(app.permission_observer.grants().is_empty());
    assert!(notice(&app).contains("Revoked: session.rs · reads ask again"));
    assert!(screen(&app).contains("Nothing remembered"));
}

#[test]
fn an_approved_outside_read_runs_inside_a_turn() {
    let directory = Directory::new();
    let other = Directory::new();
    let path = other.path().join("session.rs");
    fs::write(&path, "wall-clock times\n").unwrap();
    let read = tool_call(
        "read-1",
        "read",
        Value::object([("path", Value::string(path.to_string_lossy()))]),
    );
    let fixture = HttpFixture::new(vec![
        (200, completion("", vec![read])),
        (200, completion("Compared", vec![])),
    ]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("Compare webterminal");
    app.edited();
    assert!(!app.submit().unwrap());
    wait_until(&mut app, |app| app.state.permission.is_some());
    let pending = app.state.permission.clone().unwrap();
    assert_eq!(pending.agent, "MAIN AGENT");
    text(&mut app, "1");
    finish(&mut app);
    let requests = fixture.finish();
    let result = requests[1]
        .body
        .get("messages")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap();
    assert!(result.contains("wall-clock"), "{result}");
}

#[test]
fn the_level_changes_while_a_turn_holds_the_agent() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let agent = app.hold();
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "1");
    assert!(notice(&app).contains("Permissions: Workspace → Read · from the next call"));
    // The running agent reads the new level before its next call.
    assert_eq!(agent.permissions().level(), Level::Read);
    app.agent = Some(agent);
    app.held = None;
}

#[test]
fn permission_display_masks_secrets_and_paste_never_approves() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let secret = "permission-fixture-secret";
    app.archive.redactor.add_secret(secret.into());
    let access = outside(Kind::Runs, &format!("program --token {secret}"));
    let request = app
        .permission_observer
        .begin("sensitive", "exec", &access)
        .unwrap();
    app.poll().unwrap();
    assert!(!screen(&app).contains(secret), "{}", screen(&app));
    app.input(Decoded::Paste("2".into())).unwrap();
    assert_eq!(app.permission_observer.pending().unwrap().0.id, request.id);
    key(&mut app, 40, 0);
    app.poll().unwrap();
    assert_eq!(app.state.permission.as_ref().unwrap().focus, Some(0));
    text(&mut app, "2");
    assert_eq!(app.permission_observer.grants(), vec![access.grant()]);
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "4");
    assert!(!screen(&app).contains(secret), "{}", screen(&app));
    key(&mut app, 68, 4);
    assert!(app.permission_observer.grants().is_empty());
}

#[test]
fn multi_file_permission_masks_member_paths_without_changing_grants() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let secret = "permission-fixture-secret";
    app.archive.redactor.add_secret(secret.into());
    let members = [
        outside(Kind::Edits, &format!(r"C:\outside\{secret}\first.rs")),
        outside(Kind::Edits, r"C:\outside\second.rs"),
    ];
    let access = Access {
        target: members
            .iter()
            .map(|member| member.target.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        members: members.to_vec(),
        ..outside(Kind::Edits, "")
    };
    app.permission_observer
        .begin("edit-batch", "edit", &access)
        .unwrap();
    app.refresh_permission();
    let shown = screen(&app);
    assert!(
        shown.contains("[redacted]") && shown.contains("second.rs"),
        "{shown}"
    );
    assert!(!shown.contains(secret), "{shown}");
    assert!(
        app.permission_observer.pending().unwrap().0.access.members[0]
            .target
            .contains(secret)
    );

    text(&mut app, "2");
    assert_eq!(app.permission_observer.grants(), access.grants());
    assert!(app.permission_observer.grants()[0].target.contains(secret));
    app.dispatch("/permissions".into()).unwrap();
    text(&mut app, "4");
    assert!(!screen(&app).contains(secret), "{}", screen(&app));
}
