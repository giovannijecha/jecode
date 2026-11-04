//! A collaborator's temporary area lies deeper than its main agent's. Under a
//! long Jecode home it exceeds what Windows accepts as TEMP or TMP, where a
//! program cmd.exe starts would stay suspended until the call is cancelled.

use super::tests::create;
use super::*;
use crate::{
    backend::openrouter::OpenRouter,
    cancel::Cancellation,
    test_support::{Directory, HttpFixture, completion, tool_call},
};

#[test]
fn collaborator_under_a_long_home_starts_programs_through_cmd() {
    let project = Directory::new();
    let base = Directory::new();
    // Long enough that the collaborator's area exceeds MAX_PATH while the
    // main agent's stays within it, as in the reported benchmark home.
    let padding = 140usize
        .saturating_sub(base.path().as_os_str().len())
        .max(1);
    let home = base.path().join("h".repeat(padding));
    std::fs::create_dir(&home).unwrap();
    let fixture = HttpFixture::new(vec![
        (
            200,
            completion(
                "",
                vec![tool_call(
                    "first",
                    "bash",
                    Value::object([
                        (
                            "command",
                            Value::string(
                                "printf '%s|%s|' \"$JECODE_TMP\" \"$TMP\"; cmd.exe /C cmd.exe /C echo nested",
                            ),
                        ),
                        ("timeout_seconds", Value::number(30)),
                    ]),
                )],
            ),
        ),
        (200, completion("Report", vec![])),
    ]);
    let mut parent = Agent::new(
        OpenRouter::fixture(fixture.endpoint.clone()),
        Tools::new(project.path()).unwrap(),
    );
    parent.enable_sessions(&home).unwrap();
    parent
        .permissions()
        .set_level(crate::permissions::Level::Full)
        .unwrap();
    parent.agent_tool(&create()).unwrap();
    let observer = parent.collaboration();
    observer
        .wait(
            Some("a1"),
            std::time::Duration::from_secs(60),
            &Cancellation::default(),
        )
        .unwrap();
    let record = observer.snapshot().remove(0);
    observer.stop_all();
    fixture.finish();
    let result = record
        .get("tools")
        .and_then(Value::as_array)
        .and_then(|tools| tools.first())
        .and_then(|tool| tool.get("result"))
        .unwrap_or_else(|| panic!("{record:?}"));
    let stdout = result.get("stdout").and_then(Value::as_str).unwrap();
    let [area, inherited, output] = stdout.splitn(3, '|').collect::<Vec<_>>()[..] else {
        panic!("{result:?}");
    };
    assert!(area.encode_utf16().count() > 260, "{area}");
    assert_eq!(
        result.get("timed_out"),
        Some(&Value::Bool(false)),
        "{result:?}"
    );
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    assert_eq!(output.trim(), "nested");
    // Windows programs keep the inherited temporary directory instead.
    assert_ne!(inherited, area);
}
