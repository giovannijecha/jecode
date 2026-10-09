use crate::{
    agent::Agent,
    json::Value,
    openrouter::OpenRouter,
    sessions::{Stage, Store},
    test_support::{Directory, tool_call},
    tools::Tools,
};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn forced_process_exit_releases_lock_and_recovers_an_actual_write_with_unknown_outcome() {
    let home = Directory::new();
    let directory = Directory::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "sessions::crash_tests::crash_child",
            "--nocapture",
        ])
        .env("JECODE_CRASH_HOME", home.path())
        .env("JECODE_CRASH_DIRECTORY", directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = Running(command.spawn().unwrap());
    let marker = directory.path().join("written.txt");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "crash fixture exited before its write"
        );
        assert!(
            Instant::now() < deadline,
            "crash fixture did not reach the tool"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let store = Store::new(home.path().to_path_buf(), directory.path()).unwrap();
    let id = store.list().unwrap().sessions[0].id.clone();
    let mut resumed = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(directory.path()).unwrap(),
    );
    resumed.enable_sessions(home.path()).unwrap();
    assert!(resumed.resume(&id).unwrap_err().contains("already open"));
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let status = resumed.resume(&id).unwrap();
    assert!(status.contains("Interrupted"));
    assert_eq!(fs::read_to_string(&marker).unwrap(), "written exactly once");
    let saved = resumed.sessions().unwrap().snapshot();
    assert!(
        saved
            .messages
            .last()
            .unwrap()
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("unknown")
    );
    assert_eq!(saved.input.draft.text, "unfinished draft");
    assert_eq!(saved.input.draft.cursor, 3);
    assert_eq!(saved.input.paused.len(), 1);
    assert_eq!(saved.input.paused[0].text, "queued follow-up");
    assert_eq!(saved.input.paused[0].cursor, "queued follow-up".len());
    assert!(!saved.pending.active);
    assert!(saved.input.queued.is_empty());
    assert_eq!(fs::read_to_string(&marker).unwrap(), "written exactly once");
}

#[test]
#[ignore = "owned subprocess fixture; launched by the crash recovery test"]
fn crash_child() {
    let (Some(home), Some(directory)) = (
        std::env::var_os("JECODE_CRASH_HOME"),
        std::env::var_os("JECODE_CRASH_DIRECTORY"),
    ) else {
        return;
    };
    let home = PathBuf::from(home);
    let directory = PathBuf::from(directory);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures")
        .canonicalize()
        .unwrap();
    assert!(home.canonicalize().unwrap().starts_with(&root));
    assert!(directory.canonicalize().unwrap().starts_with(&root));
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1/chat/completions".into()),
        Tools::new(&directory).unwrap(),
    );
    agent.enable_sessions(&home).unwrap();
    agent.prepare_turn("Write the fixture").unwrap();
    let arguments = Value::object([
        ("path", Value::string("written.txt")),
        ("content", Value::string("written exactly once")),
    ]);
    agent
        .archive()
        .messages
        .lock()
        .unwrap()
        .push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::Null),
            (
                "tool_calls",
                Value::Array(vec![tool_call("write", "write", arguments.clone())]),
            ),
        ]));
    let handle = agent.sessions().unwrap();
    handle
        .checkpoint(
            &agent.archive(),
            0,
            &crate::context::Context::default(),
            Stage::Completion,
        )
        .unwrap();
    handle.input(crate::sessions::Input {
        draft: crate::sessions::Draft {
            text: "unfinished draft".into(),
            cursor: 3,
            ..Default::default()
        },
        queued: vec!["queued follow-up".into()],
        ..crate::sessions::Input::default()
    });
    handle
        .checkpoint(
            &agent.archive(),
            0,
            &crate::context::Context::default(),
            Stage::Tool("write".into()),
        )
        .unwrap();
    let result = Tools::new(&directory)
        .unwrap()
        .execute("write", &arguments.encode());
    assert!(result.get("bytes_written").is_some());
    // Deliberately stop after a real side effect and before saving its result.
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
