use super::*;

#[test]
fn explicit_backend_environment_reaches_the_supervised_child() {
    let mut command = Command::new(crate::shell::executable().unwrap());
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

#[test]
fn supervised_extensionless_program_requires_exe_even_with_script_names_on_path() {
    let directory = crate::test_support::Directory::new();
    let bin = directory.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let name = "jecode-tsc-fixture";
    std::fs::write(bin.join(name), "echo wrong > marker").unwrap();
    std::fs::write(bin.join(format!("{name}.cmd")), "echo wrong > marker").unwrap();

    let mut command = Command::new(name);
    command.current_dir(directory.path()).env("PATH", &bin);
    let result = run_observed(
        &mut command,
        None,
        Duration::from_secs(10),
        1024,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    );
    let error = match result {
        Ok(_) => panic!("script name unexpectedly launched"),
        Err(error) => error,
    };
    assert!(
        error.contains("Executable was not found on PATH"),
        "{error}"
    );
    assert!(!directory.path().join("marker").exists());

    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    std::fs::copy(system.join("hostname.exe"), bin.join(format!("{name}.exe"))).unwrap();
    let expected = Command::new(system.join("hostname.exe"))
        .output()
        .unwrap()
        .stdout;
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
    assert_eq!(output.stdout, expected);
    assert!(!directory.path().join("marker").exists());
}

#[test]
fn supervised_dotted_bare_program_prefers_exact_file_then_exe_in_each_path_entry() {
    let directory = crate::test_support::Directory::new();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let name = "jecode-python3.11";
    std::fs::copy(system.join("hostname.exe"), first.join(name)).unwrap();
    std::fs::copy(system.join("whoami.exe"), first.join(format!("{name}.exe"))).unwrap();
    std::fs::copy(system.join("whoami.exe"), second.join(name)).unwrap();
    let hostname = Command::new(system.join("hostname.exe"))
        .output()
        .unwrap()
        .stdout;
    let whoami = Command::new(system.join("whoami.exe"))
        .output()
        .unwrap()
        .stdout;
    assert_ne!(hostname, whoami);

    for (path, expected) in [
        (
            format!("{};{}", first.display(), second.display()),
            &hostname,
        ),
        (format!("{};{}", second.display(), first.display()), &whoami),
    ] {
        let mut command = Command::new(name);
        command.env("PATH", path);
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

    std::fs::remove_file(first.join(name)).unwrap();
    let mut command = Command::new(name);
    command.env("PATH", first.display().to_string());
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
    assert_eq!(output.stdout, whoami);
}
