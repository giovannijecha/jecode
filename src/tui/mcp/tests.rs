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

fn settle(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.mcp.job.is_some() && Instant::now() < deadline {
        app.poll_mcp();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.mcp.job.is_none(), "MCP UI job did not settle");
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
    app.mcp.draft = Some(saved.clone());
    app.mcp.methods = vec![AuthMethod::None, AuthMethod::Token];
    app.choose_mcp_method(0);
    assert!(app.mcp.draft.as_ref().unwrap().token.is_empty());
    assert_eq!(app.mcp.draft.as_ref().unwrap().method, AuthMethod::None);
    app.choose_mcp_method(1);
    app.state
        .selector
        .as_mut()
        .unwrap()
        .editor
        .replace("isolated-new-mcp-token".into());
    app.save_mcp_field();
    for value in ["X-Project: old", "x-project: replacement", "X-PROJECT:"] {
        app.show_mcp_field(Field::Header);
        app.state
            .selector
            .as_mut()
            .unwrap()
            .editor
            .replace(value.into());
        app.save_mcp_field();
        let headers = &app.mcp.draft.as_ref().unwrap().headers;
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
    app.mcp.draft = Some(Connection::draft(
        "GitHub",
        "https://api.githubcopilot.com/mcp/",
    ));
    app.mcp.methods = vec![AuthMethod::Token, AuthMethod::GitHubDevice];
    app.present_mcp_methods();
    let menu = app.state.selector.as_ref().unwrap();
    assert!(matches!(menu.purpose, Purpose::Mcp(Panel::Methods)));
    assert_eq!(
        menu.options
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["Token", "OAuth"]
    );
    assert_eq!(menu.options[1].fact, "Open browser");
    assert_eq!(app.mcp.draft.as_ref().unwrap().method, AuthMethod::None);
    app.show_mcp_advanced();
    assert!(
        app.state
            .selector
            .as_ref()
            .unwrap()
            .options
            .iter()
            .all(|row| !row.name.contains("client"))
    );
    app.mcp.methods = vec![AuthMethod::Token];
    app.present_mcp_methods();
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Methods)
    ));
    assert!(fixture.finish().is_empty());
}

#[test]
fn choosing_oauth_starts_authorization_without_opening_a_client_id_field() {
    let directory = Directory::new();
    let fixture = HttpFixture::new(vec![]);
    let mut app = app(&directory, &fixture);
    app.mcp.draft = Some(Connection::draft("Local", "http://127.0.0.1:1/mcp"));
    app.mcp.methods = vec![AuthMethod::Token, AuthMethod::Browser];
    app.show_mcp_methods();
    app.choose_mcp(Panel::Methods, 1);
    assert_eq!(app.mcp.draft.as_ref().unwrap().method, AuthMethod::Browser);
    assert!(app.mcp.job.is_some());
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Loading)
    ));
    assert_eq!(
        app.state.selector.as_ref().unwrap().message,
        "Starting sign-in…"
    );
    app.close_mcp_panel();
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
    let index = app
        .state
        .selector
        .as_ref()
        .unwrap()
        .options
        .iter()
        .position(|row| row.name == "Vercel")
        .unwrap();
    assert_eq!(
        app.state.selector.as_ref().unwrap().options[index].fact,
        "Client approval required"
    );
    app.choose_mcp(Panel::Add, index);
    let draft = app.mcp.draft.as_ref().unwrap();
    assert_eq!(draft.name, "Vercel");
    assert_eq!(draft.endpoint, "https://mcp.vercel.com");
    assert!(draft.token.is_empty());
    assert!(draft.oauth.is_none());
    assert!(app.mcp.methods.is_empty());
    assert!(app.mcp.job.is_none());
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Form)
    ));
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
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::List)
    ));
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
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::Detail)
    ));
    assert!(app.mcp.from_inspection);
    assert_eq!(app.selected_mcp().unwrap().name, "GitHub club");
    assert!(app.close_mcp_panel());
    assert!(matches!(
        app.state.selector.as_ref().unwrap().purpose,
        Purpose::Mcp(Panel::List)
    ));
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

#[test]
#[ignore = "paints B14 MCP panels in the native terminal renderer"]
fn native_mcp_panels_specimen() {
    use crate::tui::{state::State, view};
    let menus = [
        Selector::mcp(
            Panel::List,
            "MCP",
            Some("Settings"),
            "all conversations",
            vec![
                ("GitHub".into(), "20 tools".into()),
                ("GitHub club".into(), "suspended".into()),
                ("Supabase".into(), "sign in again".into()),
                ("Add connection".into(), String::new()),
            ],
        ),
        Selector::mcp(
            Panel::Add,
            "Add connection",
            Some("MCP"),
            "",
            vec![
                ("GitHub".into(), "api.githubcopilot.com".into()),
                ("Supabase".into(), "supabase.com".into()),
                ("Cloudflare".into(), "mcp.cloudflare.com".into()),
                ("Server URL".into(), "Any HTTP MCP server".into()),
            ],
        ),
        Selector::mcp(
            Panel::Form,
            "Connection",
            Some("MCP"),
            "all conversations",
            vec![
                ("Name".into(), "GitHub club".into()),
                (
                    "Endpoint".into(),
                    "https://api.githubcopilot.com/mcp/".into(),
                ),
                ("Sign-in".into(), "Token · entered, hidden".into()),
                ("Advanced".into(), String::new()),
                ("Save connection".into(), String::new()),
            ],
        ),
    ];
    for columns in [80, 44] {
        for menu in &menus {
            let state = State {
                width: columns,
                height: 30,
                selector: Some(Selector::mcp(
                    match &menu.purpose {
                        Purpose::Mcp(panel) => *panel,
                        _ => unreachable!(),
                    },
                    &menu.title,
                    menu.parent,
                    &menu.context,
                    menu.options
                        .iter()
                        .map(|option| (option.name.clone(), option.fact.clone()))
                        .collect(),
                )),
                ..State::default()
            };
            println!("{} · {} columns", menu.title, columns);
            for line in view::composer_frame(&state, "fixture/model", "fixture").live {
                println!("{}", line.paint(columns));
            }
        }
    }
}
