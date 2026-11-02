//! Esc paths, per-draft state and connection actions without a live server.
use super::tests::{app, draft, panel, rows, settle, type_text};
use super::*;
use crate::{
    test_support::{
        Directory, HttpFixture,
        server::{Server, accept},
    },
    tui::selector::Setting,
};
use std::io::Write;

/// A server that answers every request with 503, so checks fail quickly.
fn unavailable() -> Server<()> {
    Server::start(|listener, stop| {
        while let Ok(mut stream) = accept(&listener, &stop) {
            let _ = stream.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n");
        }
    })
}

fn open_settings(app: &mut App) {
    app.settings_parent = Some(Setting::Mcp);
    app.open_mcp(true);
    settle(app);
}

#[test]
fn esc_walks_back_one_level_and_new_drafts_start_clean() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let mut public = Connection::draft("Public", "https://public.example.test/mcp");
    public.enabled = false;
    crate::mcp::Store::new(app.mcp_home())
        .save(&public)
        .unwrap();
    open_settings(&mut app);
    app.choose_mcp(Panel::List, 0);
    assert_eq!(panel(&app), Panel::Detail(Health::Suspended));
    // A public server's methods belong to that draft only.
    app.choose_mcp(Panel::Detail(Health::Suspended), 0);
    app.mcp.draft.as_mut().unwrap().methods = vec![AuthMethod::None];
    app.mcp.draft.as_mut().unwrap().sign_in = true;
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Detail(Health::Suspended));
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::List);
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 0);
    app.choose_mcp(Panel::List, 1);
    app.choose_mcp(Panel::Add, 0);
    assert_eq!(rows(&app)[2].1, "Choose Token or OAuth");
    assert!(!app.mcp.draft.as_ref().unwrap().sign_in);
    assert!(app.mcp.selected.is_none());
    // Esc from a new form returns to Add, never a previous connection.
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Add);
    assert!(app.mcp.draft.is_none());
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::List);
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 1);
    // Server URL opens its endpoint; Esc returns to Add.
    app.choose_mcp(Panel::List, 1);
    app.choose_mcp(Panel::Add, PRESETS.len());
    assert_eq!(panel(&app), Panel::Field(Field::Endpoint));
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Add);
    app.choose_mcp(Panel::Add, PRESETS.len());
    // The form validates endpoints as the backend does.
    type_text(&mut app, "http://localhost.example.com/mcp");
    assert_eq!(panel(&app), Panel::Field(Field::Endpoint));
    type_text(&mut app, "http://[::1]:8080/mcp");
    assert_eq!(panel(&app), Panel::Form);
    // Advanced fields return to Advanced on Enter and on Esc alike.
    app.choose_mcp(Panel::Form, 3);
    app.choose_mcp(Panel::Advanced, 0);
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Advanced);
    app.choose_mcp(Panel::Advanced, 0);
    type_text(&mut app, "30");
    assert_eq!(panel(&app), Panel::Advanced);
    assert_eq!(draft(&app).timeout_secs, 30);
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Form);
    // Names are distinct before any network check.
    app.choose_mcp(Panel::Form, 0);
    type_text(&mut app, "public");
    assert_eq!(panel(&app), Panel::Field(Field::Name));
    type_text(&mut app, "Local");
    assert_eq!(panel(&app), Panel::Form);
    assert_eq!(draft(&app).name, "Local");
    app.close_mcp_panel();
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::List);
    // Leaving MCP focuses its row in Settings.
    app.close_mcp_panel();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, Purpose::Settings));
    assert_eq!(menu.selected, menu.setting_index(Setting::Mcp));
    assert!(fixture.finish().is_empty());
}

#[test]
fn suspend_resume_and_remove_update_the_connection() {
    let server = unavailable();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let store = crate::mcp::Store::new(app.mcp_home());
    let endpoint = format!("{}/mcp", server.endpoint);
    let saved = store.save(&Connection::draft("Remote", &endpoint)).unwrap();
    open_settings(&mut app);
    assert_eq!(rows(&app)[0].1, "unreachable");
    app.choose_mcp(Panel::List, 0);
    assert_eq!(panel(&app), Panel::Detail(Health::Unreachable));
    app.choose_mcp(Panel::Detail(Health::Unreachable), 3);
    assert_eq!(panel(&app), Panel::Loading);
    settle(&mut app);
    assert_eq!(panel(&app), Panel::Detail(Health::Suspended));
    assert_eq!(rows(&app)[3].0, "Resume");
    assert!(!store.list().unwrap()[0].enabled);
    // Resuming checks the connection again in the background.
    app.choose_mcp(Panel::Detail(Health::Suspended), 3);
    while app.mcp.job.is_some() {
        app.poll_mcp();
    }
    assert!(matches!(
        panel(&app),
        Panel::Detail(Health::Checking | Health::Unreachable)
    ));
    settle(&mut app);
    assert_eq!(panel(&app), Panel::Detail(Health::Unreachable));
    assert!(store.list().unwrap()[0].enabled);
    app.choose_mcp(Panel::Detail(Health::Unreachable), 4);
    assert_eq!(panel(&app), Panel::ConfirmRemove);
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Detail(Health::Unreachable));
    app.choose_mcp(Panel::Detail(Health::Unreachable), 4);
    app.choose_mcp(Panel::ConfirmRemove, 0);
    settle(&mut app);
    assert_eq!(panel(&app), Panel::List);
    assert!(app.mcp.connections.is_empty());
    assert!(store.list().unwrap().is_empty());
    assert!(!app.mcp.health.contains_key(&saved.id));
    server.finish();
    assert!(fixture.finish().is_empty());
}

#[test]
fn sign_in_from_a_connection_saves_and_esc_returns_to_it() {
    let server = unavailable();
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let store = crate::mcp::Store::new(app.mcp_home());
    let endpoint = format!("{}/mcp", server.endpoint);
    let mut connection = Connection::draft("Remote", &endpoint);
    connection.enabled = false;
    let saved = store.save(&connection).unwrap();
    app.mcp
        .method_cache
        .insert(Ui::methods_key(&saved), vec![AuthMethod::Token]);
    open_settings(&mut app);
    app.choose_mcp(Panel::List, 0);
    app.choose_mcp(Panel::Detail(Health::Suspended), 1);
    assert_eq!(panel(&app), Panel::Methods);
    assert!(app.mcp.draft.as_ref().unwrap().sign_in);
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Detail(Health::Suspended));
    assert!(app.mcp.draft.is_none());
    app.choose_mcp(Panel::Detail(Health::Suspended), 1);
    app.choose_mcp(Panel::Methods, 0);
    assert_eq!(panel(&app), Panel::Field(Field::Token));
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Methods);
    app.choose_mcp(Panel::Methods, 0);
    // Enter saves at once; a failed check returns to the token to correct.
    type_text(&mut app, "isolated-rejected-token");
    assert_eq!(panel(&app), Panel::Loading);
    settle(&mut app);
    assert_eq!(panel(&app), Panel::Field(Field::Token));
    assert!(store.list().unwrap()[0].token.is_empty());
    server.finish();
    assert!(fixture.finish().is_empty());
}

#[test]
fn a_failed_worker_returns_to_the_panel_that_started_it() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.mcp.open = true;
    app.mcp.settings = true;
    app.show_mcp_add();
    app.mcp.job = Some(Job::panicking());
    app.mcp_loading("Checking connection and loading tools…", Panel::Add);
    while app.mcp.job.is_some() {
        app.poll_mcp();
    }
    assert_eq!(panel(&app), Panel::Add);
    // Read-only inspection names no Settings parent while it loads.
    app.mcp.settings = false;
    app.mcp_loading("Checking connection and loading tools…", Panel::List);
    assert_eq!(app.state.selector.as_ref().unwrap().parent, None);
    assert!(fixture.finish().is_empty());
}
