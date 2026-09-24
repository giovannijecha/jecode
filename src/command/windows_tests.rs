//! Windows PowerShell and Win32 cwd integration tests through the real runner.
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
    let proposal = prepare(workspace, script, cwd, 30, budget).unwrap();
    run(proposal, workspace, budget, &mut |_, _| {
        ControlFlow::Continue(())
    })
    .unwrap()
}

#[test]
fn powershell_and_file_tools_share_each_selected_directory() {
    const SUB: &str = "sub's [é]";
    const INPUT: &str = "index [Ω]'s.html";
    const OUTPUT: &str = "out [é].txt";
    let parent_cwd = std::env::current_dir().unwrap();
    for (access, in_sub, absolute, root_name) in [
        (Access::Workspace, false, false, "plain root"),
        (Access::Workspace, false, false, "space's Ω"),
        (Access::Workspace, false, false, "space's [Ω]"),
        (Access::Workspace, true, false, "space's [Ω]"),
        (Access::Local, false, false, "plain root"),
        (Access::Local, false, false, "space's Ω"),
        (Access::Local, false, false, "space's [Ω]"),
        (Access::Local, true, false, "space's [Ω]"),
        (Access::Local, true, true, "space's [Ω]"),
    ] {
        let files = Fixture::new();
        let root = files.0.join(root_name);
        let sub = root.join(SUB);
        std::fs::create_dir_all(&sub).unwrap();
        let workspace = Workspace::open(&root).unwrap().with_access(access);
        let cancelled = AtomicBool::new(false);
        let budget = budget(&cancelled);
        let input = if in_sub {
            format!("{SUB}/{INPUT}")
        } else {
            INPUT.into()
        };
        let output = if in_sub {
            format!("{SUB}/{OUTPUT}")
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
            SUB
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
        let result = execute(&workspace, &script, cwd, &budget);
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
        if selected
            .to_str()
            .unwrap()
            .chars()
            .any(|c| matches!(c, '[' | ']'))
        {
            assert!(location_path.starts_with("JecodeCwd:\\"), "{location_path}");
        } else {
            assert_eq!(Path::new(location_path).canonicalize().unwrap(), expected);
        }
    }
    assert_eq!(std::env::current_dir().unwrap(), parent_cwd);
}

#[test]
fn powershell_parser_and_runtime_errors_are_plain_stderr_with_failure_status() {
    let files = Fixture::new();
    let workspace = Workspace::open(&files.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    let parser = execute(
        &workspace,
        "$x = [regex]::Matches('x', 'x') .Count",
        ".",
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

    let runtime = execute(
        &workspace,
        "Write-Output 'before'; throw 'runtime café detail'",
        ".",
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
    let proposal = prepare(
        &workspace,
        "Set-Content -LiteralPath ran.txt -Value ran",
        ".",
        20,
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
