use super::*;
use crate::{
    agent::Agent,
    backend::openrouter::OpenRouter,
    config::{Settings, Store as SettingsStore},
    session::SessionConfig,
    test_support::{Directory, HttpFixture},
    tools::Tools,
};
use std::time::{Duration, Instant};

pub(super) fn app(directory: &Directory, fixture: &HttpFixture) -> App {
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

/// Generous bound for background work on a loaded CI machine; waits end as
/// soon as their condition holds.
pub(super) const WAIT: Duration = Duration::from_secs(20);

/// Wait for the foreground job and every background check.
pub(super) fn settle(app: &mut App) {
    let deadline = Instant::now() + WAIT;
    while (app.mcp.job.is_some() || app.mcp.checks.busy()) && Instant::now() < deadline {
        app.poll_mcp();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.mcp.job.is_none(), "MCP UI job did not settle");
    assert!(!app.mcp.checks.busy(), "MCP checks did not settle");
}

pub(super) fn panel(app: &App) -> Panel {
    match app.state.selector.as_ref().map(|menu| &menu.purpose) {
        Some(Purpose::Mcp(panel)) => *panel,
        _ => panic!("no MCP panel open"),
    }
}

pub(super) fn rows(app: &App) -> Vec<(String, String)> {
    app.state
        .selector
        .as_ref()
        .unwrap()
        .options
        .iter()
        .map(|row| (row.name.clone(), row.fact.clone()))
        .collect()
}

pub(super) fn draft(app: &App) -> &Connection {
    app.mcp.connection().unwrap()
}

pub(super) fn type_text(app: &mut App, text: &str) {
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace(text.into());
    app.save_mcp_field();
}

#[test]
fn changing_auth_and_extra_headers_changes_only_the_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let mut connection = Connection::draft("GitHub", "https://api.githubcopilot.com/mcp/");
    connection.method = AuthMethod::Token;
    connection.token = "isolated-old-mcp-token".into();
    let store = crate::mcp::Store::new(app.mcp_home());
    let saved = store.save(&connection).unwrap();
    app.begin_mcp_draft(saved.clone(), true, false);
    app.mcp.draft.as_mut().unwrap().methods = vec![AuthMethod::None, AuthMethod::Token];
    app.choose_mcp_method(0);
    assert!(draft(&app).token.is_empty());
    assert_eq!(draft(&app).method, AuthMethod::None);
    app.choose_mcp_method(1);
    type_text(&mut app, "isolated-new-mcp-token");
    for value in ["X-Project: old", "x-project: replacement", "X-PROJECT:"] {
        app.show_mcp_field(Field::Header);
        type_text(&mut app, value);
        let headers = &draft(&app).headers;
        assert_eq!(headers.len(), usize::from(!value.ends_with(':')));
        if value.contains("replacement") {
            assert_eq!(headers[0].1, "replacement");
        }
    }
    assert_eq!(store.list().unwrap()[0].token, saved.token);
    assert!(store.list().unwrap()[0].headers.is_empty());
    assert!(fixture.finish().is_empty());
}

#[test]
fn sign_in_requires_an_explicit_token_or_oauth_choice() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let github = Connection::draft("GitHub", "https://api.githubcopilot.com/mcp/");
    app.begin_mcp_draft(github, false, false);
    app.mcp.draft.as_mut().unwrap().methods = vec![AuthMethod::Token, AuthMethod::GitHubDevice];
    app.present_mcp_methods();
    assert_eq!(panel(&app), Panel::Methods);
    let names = rows(&app);
    assert_eq!(names[0].0, "Token");
    assert_eq!(names[1], ("OAuth".into(), "Open browser".into()));
    assert_eq!(draft(&app).method, AuthMethod::None);
    // GitHub's OAuth client is bundled; no client row.
    app.show_mcp_advanced();
    assert!(rows(&app).iter().all(|(name, _)| !name.contains("client")));
    app.mcp.draft.as_mut().unwrap().methods = vec![AuthMethod::Token];
    app.present_mcp_methods();
    assert_eq!(panel(&app), Panel::Methods);
    assert!(fixture.finish().is_empty());
}

#[test]
fn guided_services_without_a_bundled_client_offer_a_custom_client() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    for (name, endpoint, _) in &PRESETS[1..] {
        app.begin_mcp_draft(Connection::draft(name, endpoint), false, false);
        app.show_mcp_advanced();
        assert_eq!(
            rows(&app)[2],
            ("Custom OAuth client".into(), "Automatic".into()),
            "{name}"
        );
        app.choose_mcp(Panel::Advanced, 2);
        assert_eq!(panel(&app), Panel::Field(Field::ClientId));
        type_text(&mut app, "approved-client");
        assert_eq!(panel(&app), Panel::Advanced);
        assert_eq!(rows(&app)[2].1, "Configured");
        assert_eq!(
            draft(&app).oauth_client_id.as_deref(),
            Some("approved-client")
        );
    }
    assert!(fixture.finish().is_empty());
}

#[test]
fn choosing_oauth_starts_authorization_without_opening_a_client_id_field() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.begin_mcp_draft(
        Connection::draft("Local", "http://127.0.0.1:1/mcp"),
        false,
        false,
    );
    app.mcp.draft.as_mut().unwrap().methods = vec![AuthMethod::Token, AuthMethod::Browser];
    app.show_mcp_methods();
    app.choose_mcp(Panel::Methods, 1);
    assert_eq!(draft(&app).method, AuthMethod::Browser);
    assert!(app.mcp.job.is_some());
    assert_eq!(panel(&app), Panel::Loading);
    assert_eq!(
        app.state.selector.as_ref().unwrap().message,
        "Starting sign-in…"
    );
    // Cancelling returns to the choice; the form names the unfinished sign-in.
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Methods);
    app.close_mcp_panel();
    assert_eq!(panel(&app), Panel::Form);
    assert_eq!(rows(&app)[2].1, "OAuth · not signed in");
    assert!(
        crate::mcp::Store::new(app.mcp_home())
            .list()
            .unwrap()
            .is_empty()
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn vercel_guided_setup_prefills_the_generic_form_without_connecting_or_saving() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.show_mcp_add();
    let index = rows(&app)
        .iter()
        .position(|(name, _)| name == "Vercel")
        .unwrap();
    assert_eq!(rows(&app)[index].1, "Client approval required");
    app.choose_mcp(Panel::Add, index);
    let connection = draft(&app);
    assert_eq!(connection.name, "Vercel");
    assert_eq!(connection.endpoint, "https://mcp.vercel.com");
    assert!(connection.token.is_empty());
    assert!(connection.oauth.is_none());
    assert!(app.mcp.draft.as_ref().unwrap().methods.is_empty());
    assert!(app.mcp.job.is_none());
    assert_eq!(panel(&app), Panel::Form);
    assert!(
        crate::mcp::Store::new(app.mcp_home())
            .list()
            .unwrap()
            .is_empty()
    );
    assert!(fixture.finish().is_empty());
}

#[test]
fn empty_inspection_navigates_to_settings_and_restores_the_chat_draft() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.state.editor.replace("draft before /mcp".into());
    let before = app.state.editor.clone();
    app.open_mcp(false);
    settle(&mut app);
    assert_eq!(panel(&app), Panel::List);
    assert!(app.state.selector.as_ref().unwrap().options.is_empty());
    assert!(app.mcp_empty_enter());
    assert!(app.mcp.settings);
    assert!(app.close_mcp_panel());
    assert!(!app.mcp.settings);
    assert!(app.close_mcp_panel());
    assert!(app.state.selector.is_none());
    assert_eq!(app.state.editor, before);
    assert!(fixture.finish().is_empty());
}

#[test]
fn suspended_connection_from_inspection_opens_settings_and_esc_returns() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    let home = app.mcp_home();
    let mut first = Connection::draft("GitHub", "https://api.githubcopilot.com/mcp/");
    first.enabled = false;
    crate::mcp::Store::new(home.clone()).save(&first).unwrap();
    let mut second = Connection::draft("GitHub club", "https://api.githubcopilot.com/mcp/");
    second.enabled = false;
    crate::mcp::Store::new(home).save(&second).unwrap();
    app.open_mcp(false);
    settle(&mut app);
    assert_eq!(app.mcp.connections.len(), 2);
    app.choose_mcp(Panel::List, 1);
    assert_eq!(panel(&app), Panel::Detail(Health::Suspended));
    assert!(app.mcp.from_inspection);
    assert_eq!(app.selected_mcp().unwrap().name, "GitHub club");
    assert!(app.close_mcp_panel());
    assert_eq!(panel(&app), Panel::List);
    assert_eq!(app.state.selector.as_ref().unwrap().selected, 1);
    assert!(!app.mcp.settings);
    assert!(fixture.finish().is_empty());
}

#[test]
fn native_token_field_is_masked_at_44_columns() {
    let mut menu = Selector::mcp(
        Panel::Field(Field::Token),
        "Token",
        Some("Connection"),
        "",
        vec![],
    );
    menu.editor.replace("isolated-secret-value".into());
    let state = crate::tui::state::State {
        width: 44,
        height: 30,
        selector: Some(menu),
        ..Default::default()
    };
    let frame = crate::tui::view::composer_frame(&state, "fixture/model", "fixture");
    let plain = frame
        .live
        .iter()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!plain.contains("isolated-secret-value"));
    assert!(plain.contains("Paste your token"));
    assert!(
        frame
            .live
            .iter()
            .all(|line| crate::tui::text::cells(&line.plain()) <= 44)
    );
}

/// Panels built by the same code the app uses, with fixture connections.
fn specimen_menus() -> Vec<Selector> {
    let list = menus::list(
        &[
            ("GitHub".into(), Health::Available(20)),
            ("GitHub club".into(), Health::Suspended),
            ("Supabase".into(), Health::Expired),
            ("Cloudflare".into(), Health::Checking),
        ],
        true,
    );
    let mut connection = Connection::draft("GitHub club", PRESETS[0].1);
    connection.method = AuthMethod::Token;
    connection.token = "isolated-specimen-token".into();
    vec![
        list,
        menus::add(),
        menus::form(&connection, &[AuthMethod::Token, AuthMethod::GitHubDevice]),
        menus::methods(&[AuthMethod::Token, AuthMethod::Browser]),
        menus::detail(&connection, Health::Unreachable),
    ]
}

#[test]
#[ignore = "paints B14 MCP panels in the native terminal renderer"]
fn native_mcp_panels_specimen() {
    use crate::tui::{state::State, view};
    for columns in [80, 44] {
        for menu in specimen_menus() {
            println!("{} · {} columns", menu.title, columns);
            let state = State {
                width: columns,
                height: 30,
                selector: Some(menu),
                ..State::default()
            };
            for line in view::composer_frame(&state, "fixture/model", "fixture").live {
                println!("{}", line.paint(columns));
            }
        }
    }
}
