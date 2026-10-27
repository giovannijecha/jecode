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

#[test]
fn supervised_bare_program_uses_path_order_without_current_directory() {
    let directory = crate::test_support::Directory::new();
    let cwd = directory.path().join("cwd");
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    for path in [&cwd, &first, &second] {
        std::fs::create_dir(path).unwrap();
    }
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let fixture = "jecode-path-fixture.exe";
    std::fs::copy(system.join("where.exe"), cwd.join(fixture)).unwrap();
    std::fs::copy(system.join("hostname.exe"), first.join(fixture)).unwrap();
    std::fs::copy(system.join("whoami.exe"), second.join(fixture)).unwrap();
    let expected_first = Command::new(system.join("hostname.exe"))
        .output()
        .unwrap()
        .stdout;
    let expected_second = Command::new(system.join("whoami.exe"))
        .output()
        .unwrap()
        .stdout;
    assert_ne!(expected_first, expected_second);

    for (path, expected) in [
        (
            format!("{};{}", first.display(), second.display()),
            &expected_first,
        ),
        (
            format!("{};{}", second.display(), first.display()),
            &expected_second,
        ),
    ] {
        let mut command = Command::new("jecode-path-fixture");
        command.current_dir(&cwd).env("PATH", path);
        let output = run_observed(
            &mut command,
            None,
            Duration::from_secs(10),
            4096,
            &Cancellation::default(),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(&output.stdout, expected);
    }
}
