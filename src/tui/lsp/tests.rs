use super::*;
use crate::lsp::{CheckFailure, CheckObservation};
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store as SettingsStore},
    session::SessionConfig,
    test_support::{Directory, HttpFixture},
    tools::Tools,
};

fn app(directory: &Directory, fixture: &HttpFixture) -> App {
    let settings = Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap();
    let store = SettingsStore::new(directory.path().join("config"));
    store.save(&settings).unwrap();
    App::new(
        Agent::new(
            OpenRouter::fixture(fixture.endpoint.clone()),
            Tools::new(directory.path()).unwrap(),
        ),
        SessionConfig { store, settings },
        None,
    )
}

fn press(app: &mut App, code: u16) {
    app.input(crate::tui::Decoded::Key(crate::tui::terminal::Key {
        code,
        modifiers: 0,
        character: code,
    }))
    .unwrap();
}

fn typed(app: &mut App, text: &str) {
    app.input(crate::tui::Decoded::Text(text.into())).unwrap();
}

fn panel(app: &App) -> Panel {
    match app.state.selector.as_ref().map(|menu| &menu.purpose) {
        Some(Purpose::Lsp(panel)) => *panel,
        _ => panic!("no LSP panel is open"),
    }
}

/// The latest local command result recorded in the conversation.
fn local_result(app: &App) -> Option<&str> {
    app.state.items.iter().rev().find_map(|item| match item {
        crate::tui::state::Item::Local { result, .. } => result.as_deref(),
        _ => None,
    })
}

#[test]
fn custom_server_is_created_by_typing_and_pasting_into_fields() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.open_lsp(true);
    press(&mut app, 13); // Add server
    typed(&mut app, "3"); // Custom server
    assert_eq!(panel(&app), Panel::Form);
    typed(&mut app, "1");
    assert_eq!(panel(&app), Panel::Field(Field::Name));
    typed(&mut app, "Fixtur LSPx");
    press(&mut app, 8); // Backspace
    for _ in 0..4 {
        press(&mut app, 37); // Left
    }
    typed(&mut app, "e");
    press(&mut app, 13);
    assert_eq!(panel(&app), Panel::Form);
    typed(&mut app, "2");
    app.input(crate::tui::Decoded::Paste(
        r"C:\Tools\fixture-lsp.exe".into(),
    ))
    .unwrap();
    press(&mut app, 13);
    typed(&mut app, "3");
    typed(&mut app, "fixture: .fx .fxi");
    press(&mut app, 13);
    typed(&mut app, "5"); // Advanced
    typed(&mut app, "1"); // Arguments
    typed(&mut app, r#""\\srv\share\my dir" --stdio"#);
    press(&mut app, 13);
    assert_eq!(panel(&app), Panel::Advanced);
    press(&mut app, 27);
    typed(&mut app, "7"); // Save server
    let profiles = app.lsp_store().load().unwrap();
    assert_eq!(profiles.len(), 1);
    let saved = &profiles[0];
    assert_eq!(saved.name, "Fixture LSP");
    assert_eq!(saved.executable, r"C:\Tools\fixture-lsp.exe");
    assert_eq!(saved.languages, vec!["fixture"]);
    assert_eq!(saved.extensions.len(), 2);
    assert_eq!(saved.arguments, vec![r"\\srv\share\my dir", "--stdio"]);
    assert_eq!(panel(&app), Panel::Detail);
    assert!(app.facts().lsp_runtime.statuses().is_empty());
    assert!(fixture.finish().is_empty());
}

#[test]
fn settings_changes_close_the_settings_row_as_changes() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.dispatch("/settings".into()).unwrap();
    let menu = app.state.selector.as_mut().unwrap();
    menu.selected = menu.setting_index(Setting::Lsp);
    press(&mut app, 13);
    press(&mut app, 13); // Add server
    typed(&mut app, "1"); // Guided Rust
    typed(&mut app, "4"); // Enabled: Off
    typed(&mut app, "7"); // Save server
    let notice = app.state.notice.as_ref().unwrap();
    assert_eq!(notice.text, "Language server saved · off");
    assert!(notice.change);
    assert_eq!(local_result(&app), Some("Language server saved · off"));
    typed(&mut app, "2"); // Turn on: a later change only notifies.
    assert_eq!(
        app.state.notice.as_ref().unwrap().text,
        "Language server turned on · starts when needed"
    );
    press(&mut app, 27); // Detail › LSP
    press(&mut app, 27); // LSP › Settings
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Settings
    ));
    press(&mut app, 27);
    assert_eq!(local_result(&app), Some("Language server saved · off"));
    assert!(app.lsp_store().load().unwrap()[0].enabled);
    assert!(fixture.finish().is_empty());
}

#[test]
fn saving_changed_settings_shows_the_cleared_check() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let mut profile = Profile::draft_guided_rust();
    profile.last_check = Some(CheckObservation {
        capabilities: crate::json::Value::object([]),
        root: directory.path().into(),
        executable: "rust-analyzer".into(),
        fingerprint: profile.analysis_fingerprint(),
    });
    app.lsp_store().save(profile.clone()).unwrap();
    app.open_lsp(true);
    press(&mut app, 13);
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .message
            .contains("Checked ·")
    );
    typed(&mut app, "1"); // Edit
    typed(&mut app, "2"); // Executable
    typed(&mut app, "-next");
    press(&mut app, 13);
    typed(&mut app, "7"); // Save server
    assert_eq!(panel(&app), Panel::Detail);
    let menu = app.state.selector.as_ref().unwrap();
    assert!(menu.message.contains("Not checked"), "{}", menu.message);
    assert!(app.lsp.profiles[0].last_check.is_none());
    assert_eq!(app.lsp.profiles[0].executable, "rust-analyzer-next");
    press(&mut app, 27);
    let menu = app.state.selector.as_ref().unwrap();
    assert_eq!(menu.options[0].fact, "Not checked · not started");
    assert!(fixture.finish().is_empty());
}

#[test]
fn guided_profile_is_enabled_on_draft_and_starts_nothing_on_save() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.open_lsp(true);
    app.choose_lsp(Panel::List, 0);
    app.choose_lsp(Panel::Add, 0);
    assert!(app.lsp.draft.as_ref().unwrap().enabled);
    assert_eq!(app.lsp.draft.as_ref().unwrap().executable, "rust-analyzer");
    app.choose_lsp(Panel::Form, 6);
    let profiles = app.lsp_store().load().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].last_check, None);
    assert!(app.facts().lsp_runtime.statuses().is_empty());
    assert!(fixture.finish().is_empty());
}

#[test]
fn escape_discards_draft_and_inspection_keeps_composer() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.insert("unsent draft");
    app.open_lsp(true);
    app.choose_lsp(Panel::List, 0);
    app.choose_lsp(Panel::Add, 2);
    app.lsp.draft.as_mut().unwrap().name = "Unsaved".into();
    app.close_lsp_panel();
    assert!(app.lsp.draft.is_none());
    assert!(app.lsp_store().load().unwrap().is_empty());
    app.open_lsp(false);
    assert!(app.lsp_empty_enter());
    app.close_lsp_panel();
    assert_eq!(app.state.editor.text, "unsent draft");
    assert!(fixture.finish().is_empty());
}

#[test]
fn editing_analysis_settings_invalidates_only_the_draft_check() {
    let mut profile = Profile::draft_guided_rust();
    let old = profile.clone();
    profile
        .environment
        .insert("RUST_LOG".into(), "debug".into());
    assert!(!profile.same_analysis_settings(&old));
    assert_ne!(profile.analysis_fingerprint(), old.analysis_fingerprint());
}

#[test]
fn second_guided_entry_prefills_a_distinct_name() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let first = Profile::draft_guided_rust();
    app.lsp_store().save(first.clone()).unwrap();
    app.open_lsp(true);
    app.choose_lsp(Panel::List, 1);
    app.choose_lsp(Panel::Add, 0);
    assert_eq!(
        app.lsp.draft.as_ref().unwrap().name,
        format!("{} 2", first.name)
    );
    app.close_lsp_panel();
    assert!(matches!(
        app.state.selector.as_ref().map(|menu| &menu.purpose),
        Some(Purpose::Lsp(Panel::Add))
    ));
}

#[test]
fn saved_check_failure_is_distinct_from_never_checked_in_inspection() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let mut failed = Profile::draft_guided_rust();
    failed.last_check_failure = Some(CheckFailure {
        detail: "Executable not found".into(),
        fingerprint: failed.analysis_fingerprint(),
    });
    app.lsp_store().save(failed.clone()).unwrap();
    app.open_lsp(false);
    let menu = app.state.selector.as_ref().unwrap();
    assert_eq!(menu.options[0].fact, "Executable not found");
    app.choose_lsp(Panel::List, 0);
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .message
            .contains("Executable not found")
    );
    assert!(app.facts().lsp_runtime.statuses().is_empty());
    assert!(fixture.finish().is_empty());
}

#[test]
fn late_check_result_cannot_restore_off_removed_or_reconfigured_server() {
    let directory = Directory::new();
    let provider = HttpFixture::new(vec![]);
    let mut app = app(&directory, &provider);
    let profile = Profile::draft_guided_rust();
    let observation = CheckObservation {
        capabilities: crate::json::Value::object([]),
        root: directory.path().into(),
        executable: profile.executable.clone(),
        fingerprint: profile.analysis_fingerprint(),
    };
    let store = app.lsp_store();
    store.save(profile.clone()).unwrap();
    app.open_lsp(true);
    app.lsp.selected = Some(profile.id.clone());
    store.set_enabled(&profile.id, false).unwrap();
    app.finish_lsp_check(&profile.id, observation.clone(), Panel::Detail);
    let saved = store.load().unwrap().remove(0);
    assert!(!saved.enabled);
    assert!(saved.last_check.is_some()); // Explicit Check is permitted while Off.

    store.remove(&profile.id).unwrap();
    app.finish_lsp_check(&profile.id, observation.clone(), Panel::Detail);
    assert!(store.load().unwrap().is_empty());

    store.save(profile.clone()).unwrap();
    app.open_lsp(true);
    app.lsp.selected = Some(profile.id.clone());
    let mut changed = profile.clone();
    changed.executable = "another-server".into();
    store.save(changed).unwrap();
    app.finish_lsp_check(&profile.id, observation, Panel::Detail);
    let saved = store.load().unwrap().remove(0);
    assert_eq!(saved.executable, "another-server");
    assert!(saved.last_check.is_none());
    assert!(provider.finish().is_empty());
}

#[test]
fn environment_values_are_masked_and_registered_before_a_draft_check() {
    let directory = Directory::new();
    let provider = HttpFixture::new(vec![]);
    let mut app = app(&directory, &provider);
    app.begin_lsp_draft(Profile::draft_guided_rust(), false);
    app.show_lsp_field(Field::Environment);
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace("AUTH_TOKEN=isolated-lsp-secret-value".into());
    app.save_lsp_field();
    assert_eq!(
        app.archive.redactor.text("isolated-lsp-secret-value"),
        "[redacted]"
    );
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, Purpose::Lsp(Panel::Advanced)));
    assert!(
        menu.options
            .iter()
            .any(|row| row.name == "Environment" && row.fact == "1 configured, values hidden")
    );
    assert!(
        !menu
            .options
            .iter()
            .any(|row| row.fact.contains("isolated-lsp-secret-value"))
    );
    assert!(provider.finish().is_empty());
}

#[test]
fn native_grid_renders_shared_forms_and_inspection_at_narrow_and_wide_widths() {
    use crate::tui::{render::Renderer, state::State, text, view};
    let mut profile = Profile::draft_guided_cpp();
    profile.name = "C/C++ compiler language service with a deliberately long display name".into();
    profile.last_check = Some(CheckObservation {
        capabilities: crate::json::Value::object([
            ("definitionProvider", crate::json::Value::Bool(true)),
            ("referencesProvider", crate::json::Value::Bool(true)),
            ("diagnosticProvider", crate::json::Value::Bool(true)),
        ]),
        root: PathBuf::from("C:/a project with a long nested path/source"),
        executable: "C:/Tools/clangd.exe".into(),
        fingerprint: profile.analysis_fingerprint(),
    });
    for columns in [44, 80, 120] {
        let panels = [
            menus::list(&[], &[], false),
            menus::list(&[profile.clone()], &[], true),
            menus::add(),
            menus::form(&profile),
            menus::advanced(&profile),
            menus::detail(&profile, None, true),
        ];
        for panel in panels {
            let state = State {
                width: columns,
                height: 30,
                selector: Some(panel),
                ..State::default()
            };
            let frame = view::frame(&state, "fixture/model", "project");
            assert!(
                frame
                    .live
                    .iter()
                    .all(|line| text::cells(&line.plain()) <= columns),
                "{columns} columns: {:?}",
                frame
                    .live
                    .iter()
                    .map(|line| line.plain())
                    .collect::<Vec<_>>()
            );
            let bytes = Renderer::new().paint(&frame, columns, state.height);
            assert!(!bytes.is_empty());
        }
        let state = State {
            width: columns,
            height: 30,
            selector: Some(menus::detail(&profile, None, true)),
            ..State::default()
        };
        let frame = view::frame(&state, "fixture/model", "project");
        let rows = frame
            .live
            .iter()
            .map(|line| line.plain())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rows.contains("Pull advertised"));
        assert!(rows.contains(if columns == 44 {
            "not limited by levels"
        } else {
            "not limited by Read / Workspace / Full"
        }));
    }
}
