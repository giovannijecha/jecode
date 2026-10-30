use super::tests::shell;
use super::*;

fn read(directory: &crate::test_support::Directory, name: &str) -> String {
    fs::read_to_string(directory.path().join(name)).unwrap()
}

fn helper() -> String {
    format!("'{}'", executable().unwrap().display())
}

#[test]
fn dev_null_discards_and_supplies_nothing_on_every_platform() {
    let (directory, mut shell) = shell();
    let script = "cargo --version >/dev/null 2>/dev/null && echo ok > ok; cat </dev/null > empty; echo hidden 2>/dev/null >/dev/null";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "ok"), "ok\n");
    assert_eq!(read(&directory, "empty"), "");
    assert!(!directory.path().join("dev").exists());
}

#[test]
fn recursive_copy_refuses_a_destination_inside_the_source() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("mkdir sub; touch sub/file; cp -r sub sub/inner"),
        1
    );
    assert!(!directory.path().join("sub/inner").exists());
    assert_eq!(shell.run("cp -r sub sub"), 1);
    assert_eq!(shell.run("cp -r sub copy"), 0);
    assert!(directory.path().join("copy/file").is_file());
}

#[test]
fn moving_across_volumes_copies_then_removes() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("mkdir -p tree/nested; echo data > tree/nested/file"),
        0
    );
    builtins::copy_then_remove(
        &directory.path().join("tree"),
        &directory.path().join("moved"),
    )
    .unwrap();
    assert!(!directory.path().join("tree").exists());
    assert_eq!(read(&directory, "moved/nested/file"), "data\n");
}

#[test]
fn redirection_alone_creates_or_truncates_the_file() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("echo data > kept; > kept; > created"), 0);
    assert_eq!(read(&directory, "kept"), "");
    assert_eq!(read(&directory, "created"), "");
}

#[test]
fn variable_names_follow_the_platform_environment() {
    let mut variables = Variables::default();
    variables.set("Path", "first".into());
    variables.set("PATH", "second".into());
    let entries: Vec<_> = variables.iter().collect();
    if cfg!(windows) {
        assert_eq!(entries, [("Path", "second")]);
        assert_eq!(variables.get("path"), Some("second"));
    } else {
        assert_eq!(entries.len(), 2);
        assert_eq!(variables.get("Path"), Some("first"));
    }
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("export PATH=\"$PATH\"; test -n \"$PATH\" && echo set > found"),
        0
    );
    assert_eq!(read(&directory, "found"), "set\n");
}

#[test]
fn failing_substitution_still_substitutes_and_sets_status() {
    let (directory, mut shell) = shell();
    let script =
        "x=$(printf partial; exit 3); echo \"$?:$x\" > first; echo \"$(false)done\" > second";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "first"), "3:partial\n");
    assert_eq!(read(&directory, "second"), "done\n");
    assert_eq!(shell.run("echo $(printf '%2000000s' y) > too_large"), 1);
    assert!(!directory.path().join("too_large").exists());
}

#[test]
fn assignments_prefix_pipeline_stages_background_jobs_and_builtins() {
    let (directory, mut shell) = shell();
    let script = format!(
        "X=piped {helper} -c 'echo $X' | cat > piped; X=bg {helper} -c 'echo $X' > bg & wait; X=scoped env > env; echo \"[$X]\" > after",
        helper = helper()
    );
    assert_eq!(shell.run(&script), 0);
    assert_eq!(read(&directory, "piped"), "piped\n");
    assert_eq!(read(&directory, "bg"), "bg\n");
    assert!(read(&directory, "env").contains("X=scoped\n"));
    assert_eq!(read(&directory, "after"), "[]\n");
}

#[test]
fn script_files_run_in_a_fresh_shell_with_arguments() {
    let (directory, mut shell) = shell();
    fs::write(
        directory.path().join("child.sh"),
        "echo \"$1-$2-$0\" > args\nX=inner\ncd ..\nexit 4\necho unreachable > after_exit\n",
    )
    .unwrap();
    assert_eq!(
        shell.run("X=outer; bash child.sh one two; echo \"$?:$X\" > after"),
        0
    );
    assert_eq!(read(&directory, "args"), "one-two-child.sh\n");
    assert_eq!(read(&directory, "after"), "4:outer\n");
    assert!(!directory.path().join("after_exit").exists());
}
