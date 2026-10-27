use super::*;

#[test]
fn terminal_bare_program_uses_path_order_and_dotted_fallback() {
    let root = Directory::new();
    let cwd = root.path().join("cwd");
    let first = root.path().join("first");
    let second = root.path().join("second");
    for path in [&cwd, &first, &second] {
        std::fs::create_dir(path).unwrap();
    }
    let source = root.path().join("path_fixture.cs");
    std::fs::write(
        &source,
        "using System; class Probe { public static void Main() { for (int i = 0; i < 2; i++) Console.WriteLine(\"SELECTED:\" + System.Diagnostics.Process.GetCurrentProcess().MainModule.FileName); } }",
    )
    .unwrap();
    let program = "jecode-path-fixture.exe";
    let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Add-Type -LiteralPath '{}' -OutputAssembly '{}' -OutputType ConsoleApplication",
        quote(&source),
        quote(&cwd.join(program))
    );
    assert!(
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .unwrap()
            .success()
    );
    std::fs::copy(cwd.join(program), first.join(program)).unwrap();
    std::fs::copy(cwd.join(program), second.join(program)).unwrap();
    let tools = tools(&root);
    for (path, expected) in [
        (format!("{};{}", first.display(), second.display()), &first),
        (format!("{};{}", second.display(), first.display()), &second),
    ] {
        let started = execute(
            &tools,
            Path::new("jecode-path-fixture"),
            &[],
            &[
                ("cwd", Value::string("cwd")),
                ("env", Value::object([("PATH", Value::string(path))])),
            ],
        );
        let id = text(&started, "process_id").to_owned();
        let mut output = text(&started, "stdout").to_owned();
        let ended = finish(&tools, &id, &mut output);
        let selected = expected.join(program).to_string_lossy().replace('/', "\\");
        assert_eq!(
            ended.get("exit_code").and_then(Value::as_usize),
            Some(0),
            "{ended:?}; {output:?}"
        );
        assert!(
            output.contains(&format!("SELECTED:{selected}")),
            "expected {} in PTY output: {output:?}",
            selected
        );
    }

    let dotted = "jecode-python3.11";
    std::fs::copy(cwd.join(program), first.join(dotted)).unwrap();
    std::fs::copy(cwd.join(program), first.join(format!("{dotted}.exe"))).unwrap();
    std::fs::copy(cwd.join(program), second.join(dotted)).unwrap();
    let cases = [
        (
            format!("{};{}", first.display(), second.display()),
            first.join(dotted),
        ),
        (
            format!("{};{}", second.display(), first.display()),
            second.join(dotted),
        ),
    ];
    for (path, expected) in cases {
        let started = execute(
            &tools,
            Path::new(dotted),
            &[],
            &[
                ("cwd", Value::string("cwd")),
                ("env", Value::object([("PATH", Value::string(path))])),
            ],
        );
        let id = text(&started, "process_id").to_owned();
        let mut output = text(&started, "stdout").to_owned();
        let ended = finish(&tools, &id, &mut output);
        let selected = expected.to_string_lossy().replace('/', "\\");
        assert_eq!(
            ended.get("exit_code").and_then(Value::as_usize),
            Some(0),
            "{ended:?}; {output:?}"
        );
        assert!(
            output.contains(&format!("SELECTED:{selected}")),
            "{output:?}"
        );
    }

    std::fs::remove_file(first.join(dotted)).unwrap();
    let started = execute(
        &tools,
        Path::new(dotted),
        &[],
        &[
            ("cwd", Value::string("cwd")),
            (
                "env",
                Value::object([("PATH", Value::string(first.to_string_lossy()))]),
            ),
        ],
    );
    let id = text(&started, "process_id").to_owned();
    let mut output = text(&started, "stdout").to_owned();
    let ended = finish(&tools, &id, &mut output);
    let selected = first
        .join(format!("{dotted}.exe"))
        .to_string_lossy()
        .replace('/', "\\");
    assert_eq!(
        ended.get("exit_code").and_then(Value::as_usize),
        Some(0),
        "{ended:?}; {output:?}"
    );
    assert!(
        output.contains(&format!("SELECTED:{selected}")),
        "{output:?}"
    );
}

#[test]
fn terminal_extensionless_program_requires_exe_even_with_script_names_on_path() {
    let root = Directory::new();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let name = "jecode-tsc-fixture";
    std::fs::write(bin.join(name), "echo wrong > marker").unwrap();
    std::fs::write(bin.join(format!("{name}.cmd")), "echo wrong > marker").unwrap();
    let tools = tools(&root);
    let arguments = Value::object([
        ("executable", Value::string(name)),
        ("tty", Value::Bool(true)),
        ("wait_ms", Value::number(0)),
        (
            "env",
            Value::object([("PATH", Value::string(bin.to_string_lossy()))]),
        ),
    ]);
    let failed = tools.execute("exec", &arguments.encode());
    assert_eq!(
        failed.get("outcome").and_then(Value::as_str),
        Some("unknown"),
        "{failed:?}"
    );
    assert!(failed.get("process_id").is_none());
    let stderr = tools.execute(
        "read",
        &Value::object([("path", Value::string(text(&failed, "stderr_file")))]).encode(),
    );
    assert!(
        text(&stderr, "content").contains("Executable was not found on PATH"),
        "{stderr:?}"
    );
    assert!(!root.path().join("marker").exists());

    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    std::fs::copy(system.join("hostname.exe"), bin.join(format!("{name}.exe"))).unwrap();
    let started = tools.execute("exec", &arguments.encode());
    assert!(started.get("error").is_none(), "{started:?}");
    let id = text(&started, "process_id").to_owned();
    let mut output = text(&started, "stdout").to_owned();
    let ended = finish(&tools, &id, &mut output);
    assert_eq!(ended.get("exit_code").and_then(Value::as_usize), Some(0));
    assert_eq!(text(&ended, "status"), "exited");
    assert!(!root.path().join("marker").exists());
}
