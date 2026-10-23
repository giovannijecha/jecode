use super::*;

#[test]
fn explicit_backend_environment_reaches_the_supervised_child() {
    let mut command = Command::new(crate::tools::find_bash().unwrap());
    command
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "printf '%s' \"$OPENROUTER_API_KEY\"",
        ])
        .env("OPENROUTER_API_KEY", "fixture-explicit-key");
    let output = run_observed(
        &mut command,
        None,
        Duration::from_secs(10),
        1024,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stdout, b"fixture-explicit-key");
    assert!(output.input_error.is_none());
}
