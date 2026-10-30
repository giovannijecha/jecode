use super::*;
use crate::test_support::Directory;

pub(super) fn shell() -> (Directory, Shell) {
    let directory = Directory::new();
    let mut shell = Shell::new().unwrap();
    shell.cwd = directory.path().to_path_buf();
    (directory, shell)
}

#[test]
fn quotes_variables_conditionals_and_redirection() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("A='two words'; false || printf '%s' \"$A\" > out; true && cat out > copy"),
        0
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("copy")).unwrap(),
        "two words"
    );
}

#[test]
fn pipeline_and_working_directory() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("mkdir sub; printf 'payload' | cat > sub/data; cd sub; pwd > where"),
        0
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("sub/data")).unwrap(),
        "payload"
    );
    assert!(
        fs::read_to_string(directory.path().join("sub/where"))
            .unwrap()
            .contains("sub")
    );
}

#[test]
fn unsupported_operators_fail_before_mutating_files() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("touch made; echo `pwd`"), 2);
    assert!(!directory.path().join("made").exists());
}

#[test]
fn external_command_receives_literal_argv() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("cargo --version > version"), 0);
    assert!(
        fs::read_to_string(directory.path().join("version"))
            .unwrap()
            .starts_with("cargo ")
    );
}

#[test]
fn large_pipeline_streams_between_owned_stages() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("printf '%500000s' x | wc -c > count"), 0);
    assert_eq!(
        fs::read_to_string(directory.path().join("count")).unwrap(),
        "500000\n"
    );
}

#[test]
fn text_pipeline_deletes_newlines_without_an_installed_utility() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("seq 0 3 | tr -d '\\n' > joined"), 0);
    assert_eq!(
        fs::read_to_string(directory.path().join("joined")).unwrap(),
        "0123"
    );
}

#[test]
fn conditional_failures_do_not_trigger_set_e() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("set -e; false || printf passed > status"), 0);
    assert_eq!(
        fs::read_to_string(directory.path().join("status")).unwrap(),
        "passed"
    );
}

#[test]
fn loops_and_sequence_work_after_prior_commands() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("printf start > marker; while false; do touch never; done; until true; do touch never; done; seq 1 3 > numbers"), 0);
    assert!(!directory.path().join("never").exists());
    assert_eq!(
        fs::read_to_string(directory.path().join("numbers")).unwrap(),
        "1\n2\n3\n"
    );
}

#[test]
fn background_subshell_and_wait_complete() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("(sleep 0.01; printf complete > marker) & wait"),
        0
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("marker")).unwrap(),
        "complete"
    );
}

#[test]
fn command_substitution_creates_unique_temporary_file() {
    let (directory, mut shell) = shell();
    shell
        .variables
        .set("TMPDIR", directory.path().to_string_lossy().into_owned());
    assert_eq!(
        shell
            .run("created=$(mktemp \"$TMPDIR/check.XXXXXXXXXX\"); printf '%s' \"$created\" > name"),
        0
    );
    let path = fs::read_to_string(directory.path().join("name")).unwrap();
    assert!(Path::new(&path).is_file());
}

#[test]
fn owned_shell_ignores_installed_shell_paths() {
    let (directory, _) = shell();
    let script = "printf '%s' 'owned'; test -e marker || touch marker";
    let baseline = command(script)
        .unwrap()
        .current_dir(directory.path())
        .output()
        .unwrap();
    let isolated = command(script)
        .unwrap()
        .current_dir(directory.path())
        .env("PATH", "")
        .env("JECODE_BASH", directory.path().join("missing-bash"))
        .output()
        .unwrap();
    assert_eq!(baseline.status.code(), Some(0));
    assert_eq!(isolated.status.code(), baseline.status.code());
    assert_eq!(isolated.stdout, baseline.stdout);
    assert_eq!(isolated.stderr, baseline.stderr);
}

#[test]
fn profile_flags_in_script_arguments_are_preserved() {
    let output = Command::new(executable().unwrap())
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "printf '%s' \"$1\"",
            "fixture",
            "--norc",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"--norc");
}
