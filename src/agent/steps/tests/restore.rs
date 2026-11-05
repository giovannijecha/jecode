use super::*;

#[test]
fn skipped_restore_paths_stop_the_chain_but_complete_and_noop_restores_continue() {
    for protected in [false, true] {
        let project = Directory::new();
        let home = Directory::new();
        fs::write(project.path().join("notes.txt"), "old\n").unwrap();
        fs::write(project.path().join("unprotected.txt"), "untouched\n").unwrap();
        let mut tools = Tools::new(project.path()).unwrap();
        let session = crate::checkpoints::session::Session::offline(home.path(), tools.root());
        session.begin("restore-fixture");
        tools.configure_checkpoints(session);
        let written = tools.execute(
            "write",
            &Value::object([
                ("path", Value::string("notes.txt")),
                ("content", Value::string("updated\n")),
            ])
            .encode(),
        );
        let reference = written.get("checkpoint").unwrap().get("ref").unwrap();
        let restore = step(
            "checkpoint",
            Value::object([
                ("action", Value::string("restore")),
                ("ref", reference.clone()),
                ("expected", Value::string("latest")),
                (
                    "paths",
                    Value::Array(vec![Value::string(if protected {
                        "notes.txt"
                    } else {
                        "unprotected.txt"
                    })]),
                ),
            ]),
        );
        let fixture = fixture(vec![
            restore.clone(),
            restore,
            write("after.txt", "finished"),
        ]);
        let mut agent = Agent::new(OpenRouter::fixture(fixture.endpoint.clone()), tools);
        agent
            .run_turn("Restore then continue", &mut |_| Ok(()))
            .unwrap();
        let results = results(&agent);
        let steps = entries(&results[0].1);
        assert_eq!(
            steps[0]
                .get("result")
                .unwrap()
                .get("skipped_count")
                .and_then(Value::as_usize),
            Some(usize::from(!protected))
        );
        assert_eq!(
            project.path().join("after.txt").exists(),
            protected,
            "{steps:?}"
        );
        if protected {
            assert_eq!(
                steps[1]
                    .get("result")
                    .unwrap()
                    .get("restored_count")
                    .and_then(Value::as_usize),
                Some(0),
                "restoring the same state again is a successful no-op"
            );
        } else {
            assert_eq!(
                steps[1].get("outcome").and_then(Value::as_str),
                Some("not_run")
            );
            assert_eq!(
                steps[2].get("outcome").and_then(Value::as_str),
                Some("not_run")
            );
        }
        assert_eq!(
            fs::read_to_string(project.path().join("unprotected.txt")).unwrap(),
            "untouched\n"
        );
    }
}
