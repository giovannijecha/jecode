//! Windows PowerShell and Win32 cwd integration tests through the real runner.
//! These cases share one fixture and launch helper across cwd conversion, shell
//! selection and diagnostics, so they stay together instead of duplicating setup.
use super::*;
use crate::{
    workspace::{Access, Budget},
    workspace_fixture::Fixture,
};
use std::{path::Path, sync::atomic::AtomicBool};

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(40),
    }
}

fn execute(workspace: &Workspace, script: &str, cwd: &str, budget: &Budget<'_>) -> Outcome {
    execute_with_shell(workspace, script, cwd, &Shell::default(), budget)
}

fn execute_with_shell(
    workspace: &Workspace,
    script: &str,
    cwd: &str,
    shell: &Shell,
    budget: &Budget<'_>,
) -> Outcome {
    let proposal = prepare_with_shell(workspace, script, cwd, 30, shell, budget).unwrap();
    run(proposal, workspace, budget, &mut |_, _| {
        ControlFlow::Continue(())
    })
    .unwrap()
}

fn configured_pwsh_for_tests() -> Option<Shell> {
    std::env::var("JECODE_TEST_PWSH").ok().map(|path| {
        let shell = Shell::configured(Some(&path)).unwrap();
        assert!(
            shell.label().contains("PowerShell 7.6.6"),
            "{}",
            shell.label()
        );
        shell
    })
}

fn check_file_tools_share_selected_directory(shell: &Shell, include_brackets: bool) {
    const SUB: &str = "sub's [é]";
    const INPUT: &str = "index [Ω]'s.html";
    const OUTPUT: &str = "out [é].txt";
    let parent_cwd = std::env::current_dir().unwrap();
    for (access, in_sub, absolute, root_name) in [
        (Access::Workspace, false, false, "plain root"),
        (Access::Workspace, false, false, "space's Ω"),
        (Access::Workspace, true, false, "space's Ω"),
        (Access::Workspace, false, false, "space's [Ω]"),
        (Access::Workspace, true, false, "space's [Ω]"),
        (Access::Local, false, false, "plain root"),
        (Access::Local, false, false, "space's Ω"),
        (Access::Local, true, false, "space's Ω"),
        (Access::Local, false, false, "space's [Ω]"),
        (Access::Local, true, false, "space's [Ω]"),
        (Access::Local, true, true, "space's [Ω]"),
    ] {
        if !include_brackets && root_name.contains('[') {
            continue;
        }
        let files = Fixture::new();
        let root = files.0.join(root_name);
        let sub_name = if include_brackets { SUB } else { "sub's é" };
        let sub = root.join(sub_name);
        std::fs::create_dir_all(&sub).unwrap();
        let workspace = Workspace::open(&root).unwrap().with_access(access);
        let cancelled = AtomicBool::new(false);
        let budget = budget(&cancelled);
        let input = if in_sub {
            format!("{sub_name}/{INPUT}")
        } else {
            INPUT.into()
        };
        let output = if in_sub {
            format!("{sub_name}/{OUTPUT}")
        } else {
            OUTPUT.into()
        };
        let change = workspace
            .prepare_create(&input, "from workspace\n", &budget)
            .unwrap();
        workspace.apply(change, &budget).unwrap();
        assert_eq!(workspace.read(&input, &budget).unwrap(), "from workspace\n");
        let selected = if in_sub { &sub } else { &root };
        let cwd = if absolute {
            selected.to_str().unwrap()
        } else if in_sub {
            sub_name
        } else {
            "."
        };
        let script = format!(
            "[Console]::WriteLine('location=' + (Get-Location).ProviderPath); \
             [Console]::WriteLine('location_path=' + (Get-Location).Path); \
             [Console]::WriteLine('os=' + [Environment]::CurrentDirectory); \
             [Console]::WriteLine('content=' + (Get-Content -Raw -LiteralPath '{}')); \
             [IO.File]::WriteAllText('{}', 'from shell 中文', [Text.UTF8Encoding]::new($false)); \
             {}",
            INPUT.replace('\'', "''"),
            OUTPUT.replace('\'', "''"),
            super::tests::script("success")
        );
        let result = execute_with_shell(&workspace, &script, cwd, shell, &budget);
        assert!(
            result.success(),
            "{access:?} {cwd}: {}\nstdout={}\nstderr={}",
            result.summary(),
            result.stdout,
            result.stderr
        );
        assert!(
            result.stdout.contains("content=from workspace"),
            "{}",
            result.stdout
        );
        assert!(result.stderr.contains("separate error output"));
        assert_eq!(workspace.read(&output, &budget).unwrap(), "from shell 中文");
        let expected = selected.canonicalize().unwrap();
        for label in ["location=", "os=", "cwd="] {
            let actual = result
                .stdout
                .lines()
                .find_map(|line| line.split_once(label).map(|(_, value)| value))
                .unwrap_or_else(|| panic!("missing {label} in {}", result.stdout));
            assert_eq!(
                Path::new(actual).canonicalize().unwrap(),
                expected,
                "{access:?} {cwd}: {label}{actual}"
            );
        }
        let location_path = result
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("location_path="))
            .unwrap();
        assert_eq!(Path::new(location_path).canonicalize().unwrap(), expected);
    }
    assert_eq!(std::env::current_dir().unwrap(), parent_cwd);
}

#[test]
fn powershell_and_file_tools_share_each_selected_directory() {
    check_file_tools_share_selected_directory(&Shell::default(), false);
}

#[test]
fn configured_powershell7_and_file_tools_share_each_selected_directory() {
    let Some(shell) = configured_pwsh_for_tests() else {
        return;
    };
    check_file_tools_share_selected_directory(&shell, true);
}

fn check_parser_and_runtime_diagnostics(shell: &Shell) {
    let files = Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let parser = execute_with_shell(
        &workspace,
        "$x = [regex]::Matches('x', 'x') .Count",
        ".",
        shell,
        &budget,
    );
    assert_eq!(parser.exit.unwrap().code, Some(1));
    assert_eq!(parser.stop, Stop::Exited);
    assert!(parser.stdout.is_empty(), "{}", parser.stdout);
    assert!(
        parser.stderr.contains("Unexpected token '.Count'"),
        "{}",
        parser.stderr
    );
    assert!(!parser.stderr.contains("CLIXML"), "{}", parser.stderr);
    assert!(!parser.stderr.contains("<Objs"), "{}", parser.stderr);

    let runtime = execute_with_shell(
        &workspace,
        "Write-Output 'before'; throw 'runtime café detail'",
        ".",
        shell,
        &budget,
    );
    assert_eq!(runtime.exit.unwrap().code, Some(1));
    assert_eq!(runtime.stop, Stop::Exited);
    assert!(runtime.stdout.contains("before"), "{}", runtime.stdout);
    assert!(!runtime.stdout.contains("runtime café detail"));
    assert!(
        runtime.stderr.contains("runtime café detail"),
        "{}",
        runtime.stderr
    );
    assert!(!runtime.stderr.contains("CLIXML"));
}

#[test]
fn powershell_parser_and_runtime_errors_are_plain_stderr_with_failure_status() {
    check_parser_and_runtime_diagnostics(&Shell::default());
}

#[test]
fn configured_powershell7_preserves_diagnostics_native_exit_and_cleanup() {
    let Some(shell) = configured_pwsh_for_tests() else {
        return;
    };
    check_parser_and_runtime_diagnostics(&shell);
    let failed = super::tests::launch_with_shell("failure", 20, &shell, &mut |_, _| {
        ControlFlow::Continue(())
    });
    assert_eq!(failed.exit.unwrap().code, Some(7));
    assert!(failed.stderr.contains("expected failure"));

    let mut stream = String::new();
    let cancelled = super::tests::launch_with_shell("tree", 20, &shell, &mut |_, text| {
        stream.push_str(text);
        if stream.contains("stream-ready") {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert_eq!(cancelled.stop, Stop::Cancelled);
    assert!(!cancelled.cleanup_failed);
    let start = stream.find("descendant=").unwrap() + "descendant=".len();
    let pid = stream[start..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    super::tests::assert_stopped(pid);
}

#[test]
fn powershell_preserves_literal_multiline_script_transport() {
    let files = Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let result = execute(
        &workspace,
        "Write-Output 'quotes '' $() ` café'\nWrite-Output 'second line'",
        ".",
        &budget,
    );
    assert!(result.success(), "{}", result.stderr);
    assert!(
        result.stdout.contains("quotes ' $() ` café\nsecond line"),
        "{}",
        result.stdout
    );
    let mut maximum = "Write-Output 'max transport'".to_owned();
    maximum.push_str(&" ".repeat(4096 - maximum.len()));
    let result = execute(&workspace, &maximum, ".", &budget);
    assert!(result.success(), "{}", result.stderr);
    assert!(result.stdout.contains("max transport"));
}

#[test]
fn unsupported_long_cwd_fails_before_user_script_runs() {
    let files = Fixture::new();
    let deep = files
        .0
        .join("a".repeat(90))
        .join("b".repeat(90))
        .join("c".repeat(90));
    std::fs::create_dir_all(&deep).unwrap();
    let workspace = Workspace::open(&deep).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let mut shells = vec![Shell::default()];
    if let Some(shell) = configured_pwsh_for_tests() {
        shells.push(shell);
    }
    for shell in shells {
        let proposal = prepare_with_shell(
            &workspace,
            "Set-Content -LiteralPath ran.txt -Value ran",
            ".",
            20,
            &shell,
            &budget,
        )
        .unwrap();
        let error = run(proposal, &workspace, &budget, &mut |_, _| {
            ControlFlow::Continue(())
        })
        .err()
        .expect("unsupported cwd must fail before launch");
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("MAX_PATH"), "{error}");
        assert!(!deep.join("ran.txt").exists());
    }
}

#[test]
fn converted_cwd_must_match_the_held_directory() {
    let files = Fixture::new();
    files.write("other/keep.txt", "keep");
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let mut proposal = prepare(
        &workspace,
        "Set-Content -LiteralPath ran.txt -Value ran",
        ".",
        20,
        &budget,
    )
    .unwrap();
    proposal.directory.path = files.0.join("other");
    let error = run(proposal, &workspace, &budget, &mut |_, _| {
        ControlFlow::Continue(())
    })
    .err()
    .expect("mismatched cwd must fail before launch");
    assert!(error.to_string().contains("does not match"), "{error}");
    assert!(!files.0.join("other/ran.txt").exists());
}

#[test]
fn default_powershell_rejects_bracketed_cwd_before_running() {
    for access in [Access::Workspace, Access::Local] {
        let files = Fixture::new();
        files.write("child [1]/marker.txt", "CHILD");
        let workspace = Workspace::open(&files.0).unwrap().with_access(access);
        let cancelled = AtomicBool::new(false);
        let budget = budget(&cancelled);
        let result = prepare_with_shell(
            &workspace,
            "Set-Content -LiteralPath ran.txt -Value ran",
            "child [1]",
            20,
            &Shell::default(),
            &budget,
        );
        let error = result
            .err()
            .expect("unsafe cwd must be rejected before approval");
        assert!(error.contains("windows_powershell_executable"), "{error}");
        assert!(!files.0.join("child [1]/ran.txt").exists());
    }
}

#[test]
fn unverified_powershell7_version_rejects_bracketed_cwd() {
    let files = Fixture::new();
    files.write("child [1]/marker.txt", "CHILD");
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let shell = Shell::PowerShell7 {
        executable: files.0.join("unused-pwsh.exe"),
        version: "7.5.7".into(),
    };
    let error = prepare_with_shell(
        &workspace,
        "Write-Output safe",
        "child [1]",
        20,
        &shell,
        &budget,
    )
    .err()
    .expect("unverified shell must not run from bracketed cwd");
    assert!(error.contains("7.6.6"), "{error}");
}

#[test]
fn configured_shell_rejects_missing_invalid_and_non_powershell7_executables() {
    let invalid = Shell::configured(Some("pwsh.exe")).unwrap_err();
    assert!(invalid.to_string().contains("absolute local"), "{invalid}");
    let files = Fixture::new();
    let missing = files.0.join("missing/pwsh.exe");
    let missing = Shell::configured(Some(missing.to_str().unwrap())).unwrap_err();
    assert!(missing.to_string().contains("unavailable"), "{missing}");
    let system = Path::new(&std::env::var_os("WINDIR").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let wrong_version = Shell::configured(Some(system.to_str().unwrap())).unwrap_err();
    assert!(
        wrong_version
            .to_string()
            .contains("did not report PowerShell 7"),
        "{wrong_version}"
    );
}

#[test]
fn bracket_cwd_preserves_relative_parent_and_changed_directory_semantics() {
    let Some(shell) = configured_pwsh_for_tests() else {
        return;
    };
    for access in [Access::Workspace, Access::Local] {
        let files = Fixture::new();
        files.write("marker.txt", "PARENT");
        files.write("child [1]/marker.txt", "CHILD");
        let workspace = Workspace::open(&files.0).unwrap().with_access(access);
        let cancelled = AtomicBool::new(false);
        let budget = budget(&cancelled);
        let script = format!(
            "[Console]::WriteLine('child=' + (Get-Content -Raw -LiteralPath 'marker.txt')); \
             Set-Content -LiteralPath 'made-in-child.txt' -Value CHILD-WRITE -Encoding UTF8; \
             [Console]::WriteLine('relative-parent=' + (Get-Content -Raw -LiteralPath '../marker.txt')); \
             Set-Location -LiteralPath '..'; \
             [Console]::WriteLine('after-cd=' + (Get-Location).Path); \
             [Console]::WriteLine('after-read=' + (Get-Content -Raw -LiteralPath 'marker.txt')); \
             Set-Content -LiteralPath 'made-in-parent.txt' -Value PARENT-WRITE -Encoding UTF8; \
             {}",
            super::tests::script("read-marker")
        );
        let result = execute_with_shell(&workspace, &script, "child [1]", &shell, &budget);
        assert!(
            result.success(),
            "{access:?}: {}\nstdout={}\nstderr={}",
            result.summary(),
            result.stdout,
            result.stderr
        );
        for expected in [
            "child=CHILD",
            "relative-parent=PARENT",
            "after-read=PARENT",
            "marker=PARENT",
        ] {
            assert!(
                result.stdout.contains(expected),
                "{access:?} {expected}: {}",
                result.stdout
            );
        }
        for label in ["after-cd=", "cwd="] {
            let actual = result
                .stdout
                .lines()
                .find_map(|line| line.split_once(label).map(|(_, path)| path))
                .unwrap_or_else(|| panic!("missing {label}: {}", result.stdout));
            assert_eq!(
                Path::new(actual).canonicalize().unwrap(),
                files.0.canonicalize().unwrap()
            );
        }
        assert!(
            workspace
                .read("child [1]/made-in-child.txt", &budget)
                .unwrap()
                .contains("CHILD-WRITE")
        );
        assert!(files.0.join("made-in-parent.txt").exists());
        assert!(!files.0.join("child [1]/made-in-parent.txt").exists());
    }
}

#[test]
fn bracket_cwd_pwd_paths_are_usable_by_native_file_apis() {
    let Some(shell) = configured_pwsh_for_tests() else {
        return;
    };
    for access in [Access::Workspace, Access::Local] {
        let files = Fixture::new();
        files.write("child [1]/marker.txt", "CHILD");
        let workspace = Workspace::open(&files.0).unwrap().with_access(access);
        let cancelled = AtomicBool::new(false);
        let budget = budget(&cancelled);
        let result = execute_with_shell(
            &workspace,
            "[Console]::WriteLine('pwd=' + (Get-Location).Path); \
             [Console]::WriteLine('via-pwd=' + [IO.File]::ReadAllText((Join-Path $PWD.Path 'marker.txt'))); \
             [Console]::WriteLine('via-location=' + [IO.File]::ReadAllText((Join-Path (Get-Location).Path 'marker.txt')))",
            "child [1]",
            &shell,
            &budget,
        );
        assert!(
            result.success(),
            "{access:?}: {}\nstdout={}\nstderr={}",
            result.summary(),
            result.stdout,
            result.stderr
        );
        assert!(result.stdout.contains("via-pwd=CHILD"), "{}", result.stdout);
        assert!(
            result.stdout.contains("via-location=CHILD"),
            "{}",
            result.stdout
        );
        let pwd = result
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("pwd="))
            .unwrap();
        assert_eq!(
            Path::new(pwd).canonicalize().unwrap(),
            files.0.join("child [1]").canonicalize().unwrap()
        );
    }
}
