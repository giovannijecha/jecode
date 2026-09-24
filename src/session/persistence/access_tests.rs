//! External file access and effects use fake homes and an inert model backend.
use super::*;
use crate::{
    providers::openai_account::{Progress, Request, Response, Status, client},
    session::{
        self, End, Event, Session,
        tool_tests::{call, calls_response},
    },
    tls::Budget,
    workspace_fixture::Fixture,
};
use std::{
    ops::ControlFlow,
    sync::{Arc, Mutex},
};

struct Backend {
    requests: Arc<Mutex<Vec<String>>>,
    tools: bool,
}
impl session::worker::Backend for Backend {
    fn login(
        &mut self,
        _: &Budget<'_>,
        _: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<(), client::Error> {
        Ok(())
    }
    fn generate(
        &mut self,
        request: &Request,
        _: &Budget<'_>,
        _: &mut dyn FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<Response, client::Error> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.encode(LIMIT)?);
        Ok(if self.tools && requests.len() == 1 {
            let command = json::encode(
                &json::object([
                    ("command", text(&crate::command::tests::script("write"))),
                    ("path", text("../b")),
                ]),
                8192,
            )
            .unwrap();
            calls_response(vec![
                call("read", "read_file", r#"{"path":"../b/file"}"#),
                call(
                    "edit",
                    "edit_file",
                    r#"{"path":"../b/file","old_text":"old","new_text":"new"}"#,
                ),
                call("run", "run_command", &command),
                call(
                    "verify",
                    "read_file",
                    r#"{"path":"../b/command-result.txt"}"#,
                ),
            ])
        } else {
            session::tests::response("Observed the recorded results.", Status::Completed)
        })
    }
}

#[test]
fn direct_external_work_survives_resume_without_replay() {
    {
        let home = crate::state::tests::Fixture::new();
        let Some(store) = home.store() else {
            return;
        };
        let files = Fixture::new();
        files.write("a/readme", "A");
        files.write("b/file", "old\n");
        let workspace = Workspace::open(&files.0.join("a"))
            .unwrap()
            .with_access(Access::Local);
        let history = create(&store, Model::Luna, Some(&workspace)).unwrap();
        let id = history.record.as_ref().unwrap().id.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut run = Session::with_history(
            Model::Luna,
            Backend {
                requests: requests.clone(),
                tools: true,
            },
            Some(workspace),
            history,
        )
        .unwrap();
        assert!(matches!(
            session::tests::next(&mut run),
            Event::Restored { .. }
        ));
        assert!(matches!(session::tests::next(&mut run), Event::Ready));
        assert!(run.submit("Read, edit and verify project B"));
        let mut edits = 0;
        let mut commands = 0;
        loop {
            match session::command_tests::next(&mut run) {
                Event::EditPlanned { preview, .. } => {
                    edits += 1;
                    assert!(Path::new(&preview.path).is_absolute());
                }
                Event::CommandPlanned { preview, .. } => {
                    commands += 1;
                    assert_eq!(
                        Path::new(&preview.cwd).canonicalize().unwrap(),
                        files.0.join("b").canonicalize().unwrap()
                    );
                }
                Event::Finished(end, metrics) => {
                    assert_eq!(end, End::Complete);
                    assert_eq!(metrics.tool_calls, 4);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(edits, 1);
        assert_eq!(commands, 1);
        assert_eq!(
            std::fs::read_to_string(files.0.join("b/file")).unwrap(),
            "new\n"
        );
        assert!(files.0.join("b/command-result.txt").exists());
        assert!(!files.0.join("a/command-result.txt").exists());
        assert!(requests.lock().unwrap()[0].contains("File access: local"));
        drop(run);
        let saved = load(&store, &id, true).unwrap();
        assert_eq!(saved.access, Access::Local);
        let workspace = Workspace::open(saved.workspace.as_ref().unwrap())
            .unwrap()
            .with_access(saved.access);
        let continued = Arc::new(Mutex::new(Vec::new()));
        let mut run = Session::with_history(
            saved.model,
            Backend {
                requests: continued.clone(),
                tools: false,
            },
            Some(workspace),
            saved.history,
        )
        .unwrap();
        assert!(matches!(
            session::tests::next(&mut run),
            Event::Restored { turns: 1, .. }
        ));
        assert!(matches!(session::tests::next(&mut run), Event::Ready));
        assert!(continued.lock().unwrap().is_empty());
        assert!(run.submit("Report the previous results only"));
        loop {
            match session::tests::next(&mut run) {
                Event::Finished(End::Complete, _) => break,
                Event::EditPlanned { .. } | Event::CommandPlanned { .. } => {
                    panic!("historical effect replayed")
                }
                _ => {}
            }
        }
        assert_eq!(continued.lock().unwrap().len(), 1);
        drop(run);
    }
}

#[test]
fn saved_access_is_authoritative_and_missing_legacy_access_stays_bounded() {
    let home = crate::state::tests::Fixture::new();
    let Some(store) = home.store() else {
        return;
    };
    let workspace = Workspace::open(&home.0).unwrap().with_access(Access::Local);
    let history = create(&store, Model::Luna, Some(&workspace)).unwrap();
    let id = history.record.as_ref().unwrap().id.clone();
    drop(history);
    let saved = load(&store, &id, true).unwrap();
    let wrong = Workspace::open(&home.0).unwrap();
    let directory = crate::session::scope::Directory::open(&home.0).unwrap();
    assert!(Session::resume(saved, &directory, Some(wrong)).is_err()); // Fails before any real login.
    let sessions = store.directory("sessions").unwrap();
    let body = sessions
        .read(&format!("{id}.json"), LIMIT)
        .unwrap()
        .unwrap();
    let mut value = json::parse(&body, Default::default()).unwrap();
    let Value::Object(fields) = &mut value else {
        panic!()
    };
    fields.remove("file_access");
    sessions
        .replace(&format!("{id}.json"), &json::encode(&value, LIMIT).unwrap())
        .unwrap();
    let saved = load(&store, &id, true).unwrap();
    assert_eq!(saved.access, Access::Workspace);
    drop(saved);
    let Value::Object(fields) = &mut value else {
        panic!()
    };
    fields.insert("file_access".into(), text("unknown"));
    let invalid = json::encode(&value, LIMIT).unwrap();
    sessions.replace(&format!("{id}.json"), &invalid).unwrap();
    assert!(load(&store, &id, true).is_err());
    assert_eq!(
        sessions
            .read(&format!("{id}.json"), LIMIT)
            .unwrap()
            .unwrap(),
        invalid
    );
}
