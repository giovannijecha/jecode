use super::*;

#[test]
fn terminal_bare_program_uses_path_order_without_current_directory() {
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
}
