use super::*;
use crate::{
    lsp::{Store, fixture::Fixture},
    permissions::Level,
    test_support::{Directory, HttpFixture, completion, tool_call},
};
use std::path::Path;

fn definition(path: &Path) -> Value {
    Value::object([
        ("action", Value::string("definition")),
        ("path", Value::string(path.to_string_lossy())),
        ("line", Value::number(1)),
        ("column", Value::number(1)),
    ])
}

fn responses(path: &Path, ids: &[&str]) -> Vec<(u16, Value)> {
    ids.iter()
        .flat_map(|id| {
            [
                (
                    200,
                    completion("", vec![tool_call(id, "lsp", definition(path))]),
                ),
                (200, completion("LSP result received", vec![])),
            ]
        })
        .collect()
}

fn result(agent: &Agent, id: &str) -> Value {
    let messages = agent.archive().messages.lock().unwrap().clone();
    let content = messages
        .iter()
        .find(|message| message.get("tool_call_id").and_then(Value::as_str) == Some(id))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("LSP tool response was recorded");
    crate::json::parse(content).unwrap()
}

#[test]
fn enabled_lsp_reaches_the_server_at_every_local_permission_level() {
    let server = Fixture::new();
    let home = Directory::new();
    let profile = server.profile();
    Store::new(home.path().to_path_buf())
        .save(profile.clone())
        .unwrap();
    let ids = ["lsp-read", "lsp-workspace", "lsp-full"];
    let model = HttpFixture::new(responses(&server.sample(), &ids));
    let mut agent = Agent::new(
        OpenRouter::fixture(model.endpoint.clone()),
        Tools::new(server.workspace()).unwrap(),
    );
    agent.configure_lsp(home.path()).unwrap();

    for (level, id) in Level::ALL.into_iter().zip(ids) {
        agent.permissions().set_level(level).unwrap();
        let mut started = false;
        agent
            .run_turn("Find the sample definition", &mut |event| {
                if let Event::ToolStarted {
                    name, arguments, ..
                } = &event
                    && name == "lsp"
                {
                    assert_eq!(
                        arguments.get("server_name").and_then(Value::as_str),
                        Some(profile.name.as_str())
                    );
                    started = true;
                }
                assert!(!matches!(event, Event::PermissionRequest { .. }));
                Ok(())
            })
            .unwrap();
        assert!(started, "LSP was not dispatched at {level:?}");
        let reply = result(&agent, id);
        assert_eq!(reply.get("status").and_then(Value::as_str), Some("ready"));
        let locations = reply.get("locations").and_then(Value::as_array).unwrap();
        assert_eq!(locations.len(), 1);
        let returned_path = locations[0].get("path").and_then(Value::as_str).unwrap();
        assert_eq!(
            std::fs::canonicalize(returned_path).unwrap(),
            std::fs::canonicalize(server.sample()).unwrap()
        );
        let start = locations[0].get("start").unwrap();
        assert_eq!(start.get("line"), Some(&Value::number(1)));
        assert_eq!(start.get("column"), Some(&Value::number(1)));
        assert!(server.marker().exists());
    }
    assert_eq!(model.finish().len(), 6);
}

#[test]
fn disabled_lsp_returns_off_without_launch_at_every_local_permission_level() {
    let server = Fixture::new();
    let home = Directory::new();
    let mut profile = server.profile();
    profile.enabled = false;
    Store::new(home.path().to_path_buf()).save(profile).unwrap();
    let ids = ["off-read", "off-workspace", "off-full"];
    let model = HttpFixture::new(responses(&server.sample(), &ids));
    let mut agent = Agent::new(
        OpenRouter::fixture(model.endpoint.clone()),
        Tools::new(server.workspace()).unwrap(),
    );
    agent.configure_lsp(home.path()).unwrap();

    for (level, id) in Level::ALL.into_iter().zip(ids) {
        agent.permissions().set_level(level).unwrap();
        agent
            .run_turn("Find the sample definition", &mut |event| {
                assert!(!matches!(event, Event::PermissionRequest { .. }));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            result(&agent, id).get("status").and_then(Value::as_str),
            Some("off")
        );
        assert!(!server.marker().exists());
        assert!(agent.lsp_runtime().statuses().is_empty());
    }
    assert_eq!(model.finish().len(), 6);
}
