use super::*;

#[test]
fn cancellation_between_steps_preserves_the_result_and_skips_later_writes() {
    let project = Directory::new();
    let home = Directory::new();
    let fixture = HttpFixture::new(vec![(
        200,
        completion(
            "",
            vec![tool_call(
                "chain",
                "steps",
                Value::object([(
                    "steps",
                    Value::Array(vec![write("a.txt", "first"), write("b.txt", "never")]),
                )]),
            )],
        ),
    )]);
    let mut agent = agent(&project, &fixture);
    agent.enable_sessions(home.path()).unwrap();
    let handle = agent.sessions().unwrap();
    let cancellation = agent.cancellation();
    let failure = agent
        .run_turn("Do it", &mut |event| {
            if let crate::events::Event::ToolFinished { id, .. } = event
                && id == "chain_s1"
            {
                let between = handle.snapshot();
                assert_eq!(between.pending.tool.as_deref(), Some("chain"));
                assert!(
                    between.messages.iter().all(|message| {
                        message.get("tool_call_id").and_then(Value::as_str) != Some("chain")
                    }),
                    "the unfinished call remains pending for crash recovery"
                );
                cancellation.cancel();
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(failure, "Operation cancelled");
    let results = results(&agent);
    let steps = entries(&results[0].1);
    assert!(steps[0].get("result").unwrap().get("error").is_none());
    assert_eq!(
        steps[1].get("outcome").and_then(Value::as_str),
        Some("not_run")
    );
    assert_eq!(
        fs::read_to_string(project.path().join("a.txt")).unwrap(),
        "first"
    );
    assert!(!project.path().join("b.txt").exists());
    assert_eq!(fixture.finish().len(), 1);
    assert!(
        handle.snapshot().messages.iter().any(|message| {
            message.get("tool_call_id").and_then(Value::as_str) == Some("chain")
        }),
        "the combined receipt is saved after cancellation"
    );
}
