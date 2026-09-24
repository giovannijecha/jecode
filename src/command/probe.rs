//! A bounded, read/write capability check against an owned temporary directory.
//! It uses the same proposal, wrapper, process job and capture path as commands.
use super::{
    Outcome, Shell, Stop, prepare_with_shell, run,
    selection::{BracketCwd, ProbeIssue},
};
use crate::workspace::{Budget, Workspace};
use std::{
    fs, io,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

const SCRIPT: &str = r#"
[Console]::WriteLine('JECODE-CWD-PROBE-1')
[Console]::WriteLine('child=' + (Get-Content -Raw -LiteralPath 'marker.txt'))
Set-Content -LiteralPath 'child-written.txt' -Value 'CHILD-WRITTEN' -Encoding utf8
[Console]::WriteLine('parent=' + (Get-Content -Raw -LiteralPath '../marker.txt'))
[Console]::WriteLine('pwd=' + $PWD.Path)
[Console]::WriteLine('location=' + (Get-Location).Path)
[Console]::WriteLine('native-pwd=' + [IO.File]::ReadAllText([IO.Path]::Combine($PWD.Path, 'marker.txt')))
[Console]::WriteLine('native-location=' + [IO.File]::ReadAllText([IO.Path]::Combine((Get-Location).Path, 'marker.txt')))
Set-Location -LiteralPath '..'
[Console]::WriteLine('after=' + (Get-Location).Path)
[Console]::WriteLine('after-read=' + (Get-Content -Raw -LiteralPath 'marker.txt'))
Set-Content -LiteralPath 'parent-written.txt' -Value 'PARENT-WRITTEN' -Encoding utf8
[Console]::WriteLine('native-after=' + [IO.File]::ReadAllText([IO.Path]::Combine((Get-Location).Path, 'marker.txt')))
# The absolute Windows system cmd.exe checks the native child cwd after cd.
$nativeCwd = & ([IO.Path]::Combine([Environment]::SystemDirectory, 'cmd.exe')) /d /c cd
[Console]::WriteLine('native-child=' + $nativeCwd)
"#;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) fn check(shell: &Shell, workspace: Option<&Path>) -> BracketCwd {
    check_with(&std::env::temp_dir(), workspace, |fixture| {
        let workspace = Workspace::open(&fixture.root).map_err(|_| ProbeIssue::Fixture)?;
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(6),
        };
        let proposal = prepare_with_shell(&workspace, SCRIPT, "child [1]", 5, shell, &budget)
            .map_err(|_| ProbeIssue::Fixture)?;
        let mut streamed = 0usize;
        let mut exceeded = false;
        let outcome = run(proposal, &workspace, &budget, &mut |_, text| {
            streamed = streamed.saturating_add(text.len());
            if streamed > 8192 {
                exceeded = true;
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .map_err(|_| ProbeIssue::Launch)?;
        if outcome.cleanup_failed {
            Err(ProbeIssue::Cleanup)
        } else if exceeded {
            Err(ProbeIssue::Output)
        } else {
            Ok(outcome)
        }
    })
}

fn check_with(
    base: &Path,
    workspace: Option<&Path>,
    execute: impl FnOnce(&Fixture) -> Result<Outcome, ProbeIssue>,
) -> BracketCwd {
    let mut fixture = match Fixture::create(base, workspace) {
        Ok(fixture) => fixture,
        Err(issue) => return BracketCwd::Inconclusive(issue),
    };
    let result = match execute(&fixture) {
        Ok(outcome) => inspect(&fixture, &outcome),
        Err(issue) => BracketCwd::Inconclusive(issue),
    };
    if fixture.cleanup().is_err() {
        BracketCwd::Inconclusive(ProbeIssue::Cleanup)
    } else {
        result
    }
}

fn inspect(fixture: &Fixture, outcome: &Outcome) -> BracketCwd {
    if outcome.cleanup_failed {
        return BracketCwd::Inconclusive(ProbeIssue::Cleanup);
    }
    if outcome.truncated || outcome.stop == Stop::OutputLimit || outcome.stop == Stop::IoError {
        return BracketCwd::Inconclusive(ProbeIssue::Output);
    }
    if outcome.stop == Stop::Timeout {
        return BracketCwd::Inconclusive(ProbeIssue::Timeout);
    }
    if outcome.stop == Stop::Exited && outcome.exit.is_some_and(|exit| exit.code != Some(0)) {
        return BracketCwd::Incompatible;
    }
    if !outcome.success() || !outcome.stderr.is_empty() {
        return BracketCwd::Inconclusive(ProbeIssue::Malformed);
    }
    let lines: Vec<_> = outcome.stdout.lines().collect();
    let keys = [
        "child",
        "parent",
        "pwd",
        "location",
        "native-pwd",
        "native-location",
        "after",
        "after-read",
        "native-after",
        "native-child",
    ];
    if lines.first() != Some(&"JECODE-CWD-PROBE-1") || lines.len() != keys.len() + 1 {
        return BracketCwd::Inconclusive(ProbeIssue::Malformed);
    }
    let mut values = Vec::with_capacity(keys.len());
    for (line, key) in lines[1..].iter().zip(keys) {
        let Some(value) = line
            .strip_prefix(key)
            .and_then(|text| text.strip_prefix('='))
        else {
            return BracketCwd::Inconclusive(ProbeIssue::Malformed);
        };
        values.push(value);
    }
    let child = fixture.root.join("child [1]");
    let matches_path = |actual: &str, expected: &Path| {
        Path::new(actual)
            .canonicalize()
            .is_ok_and(|path| path == expected)
    };
    let child_write = fs::read_to_string(child.join("child-written.txt"))
        .is_ok_and(|text| text.trim_start_matches('\u{feff}').trim() == "CHILD-WRITTEN");
    let parent_write = fs::read_to_string(fixture.root.join("parent-written.txt"))
        .is_ok_and(|text| text.trim_start_matches('\u{feff}').trim() == "PARENT-WRITTEN");
    if values[0] == "CHILD"
        && values[1] == "PARENT"
        && matches_path(values[2], &child)
        && matches_path(values[3], &child)
        && values[4] == "CHILD"
        && values[5] == "CHILD"
        && matches_path(values[6], &fixture.root)
        && values[7] == "PARENT"
        && values[8] == "PARENT"
        && matches_path(values[9], &fixture.root)
        && child_write
        && parent_write
        && !child.join("parent-written.txt").exists()
        && !fixture.root.join("child-written.txt").exists()
    {
        BracketCwd::Supported
    } else {
        BracketCwd::Incompatible
    }
}

struct Fixture {
    root: PathBuf,
    base: PathBuf,
    identity: Option<PathBuf>,
    cleaned: bool,
}
impl Fixture {
    fn create(base: &Path, selected_directory: Option<&Path>) -> Result<Self, ProbeIssue> {
        let base = base.canonicalize().map_err(|_| ProbeIssue::Fixture)?;
        if !base.is_dir()
            || selected_directory.is_some_and(|path| {
                path.canonicalize()
                    .map_or(true, |selected| base.starts_with(selected))
            })
        {
            return Err(ProbeIssue::Fixture);
        }
        for _ in 0..16 {
            let root = base.join(format!(
                "jecode-shell-probe-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let mut fixture = Self {
                        root,
                        base,
                        identity: None,
                        cleaned: false,
                    };
                    let setup = (|| {
                        let identity = fixture
                            .root
                            .canonicalize()
                            .map_err(|_| ProbeIssue::Fixture)?;
                        if identity.parent() != Some(fixture.base.as_path()) {
                            return Err(ProbeIssue::Fixture);
                        }
                        fixture.identity = Some(identity);
                        fs::create_dir(fixture.root.join("child [1]"))
                            .map_err(|_| ProbeIssue::Fixture)?;
                        fs::write(fixture.root.join("marker.txt"), "PARENT")
                            .map_err(|_| ProbeIssue::Fixture)?;
                        fs::write(fixture.root.join("child [1]/marker.txt"), "CHILD")
                            .map_err(|_| ProbeIssue::Fixture)?;
                        Ok(())
                    })();
                    if setup.is_err() {
                        return Err(if fixture.cleanup().is_ok() {
                            ProbeIssue::Fixture
                        } else {
                            ProbeIssue::Cleanup
                        });
                    }
                    return Ok(fixture);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(ProbeIssue::Fixture),
            }
        }
        Err(ProbeIssue::Fixture)
    }
    fn cleanup(&mut self) -> Result<(), ProbeIssue> {
        if self.cleaned {
            return Ok(());
        }
        // The root is a newly created direct child of the canonical temp base.
        // Recheck its resolved identity before recursively deleting anything.
        let current = self.root.canonicalize().map_err(|_| ProbeIssue::Cleanup)?;
        if current.parent() != Some(self.base.as_path())
            || self
                .identity
                .as_ref()
                .is_some_and(|identity| identity != &current)
        {
            return Err(ProbeIssue::Cleanup);
        }
        fs::remove_dir_all(&self.root).map_err(|_| ProbeIssue::Cleanup)?;
        self.cleaned = true;
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{command::Exit, workspace::Access};
    use std::{cell::Cell, sync::atomic::AtomicBool};

    fn outcome(stop: Stop, stdout: String) -> Outcome {
        Outcome {
            stop,
            exit: Some(Exit {
                code: Some(0),
                signal: None,
            }),
            stdout,
            stderr: String::new(),
            truncated: false,
            bytes: 0,
            elapsed_ms: 0,
            cleanup_failed: false,
        }
    }
    fn synthetic_success(fixture: &Fixture) -> String {
        fs::write(
            fixture.root.join("child [1]/child-written.txt"),
            "CHILD-WRITTEN",
        )
        .unwrap();
        fs::write(fixture.root.join("parent-written.txt"), "PARENT-WRITTEN").unwrap();
        let child = fixture.root.join("child [1]");
        format!(
            "JECODE-CWD-PROBE-1\nchild=CHILD\nparent=PARENT\npwd={}\nlocation={}\nnative-pwd=CHILD\nnative-location=CHILD\nafter={}\nafter-read=PARENT\nnative-after=PARENT\nnative-child={}\n",
            child.display(),
            child.display(),
            fixture.root.display(),
            fixture.root.display()
        )
    }
    fn assert_blocks_user_script(capability: BracketCwd, expected: &str) {
        let fixture = crate::workspace_fixture::Fixture::new();
        fixture.write("child [1]/marker.txt", "CHILD");
        let workspace = Workspace::open(&fixture.0).unwrap();
        let shell = Shell::PowerShell7 {
            executable: PathBuf::from("unused-pwsh.exe"),
            version: "7.6.6".into(),
            bracket_cwd: capability,
        };
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        };
        let error = prepare_with_shell(
            &workspace,
            "Set-Content -LiteralPath ran.txt -Value ran",
            "child [1]",
            5,
            &shell,
            &budget,
        )
        .err()
        .expect("the user script must be rejected before launch");
        assert!(error.contains(expected), "{error}");
        assert!(!fixture.0.join("child [1]/ran.txt").exists());
    }

    #[test]
    fn successful_semantics_are_cached_for_later_proposals() {
        let calls = Cell::new(0);
        let result = check_with(&std::env::temp_dir(), None, |fixture| {
            calls.set(calls.get() + 1);
            Ok(outcome(Stop::Exited, synthetic_success(fixture)))
        });
        assert_eq!(result, BracketCwd::Supported);
        // The version is synthetic metadata, not a live validation of this release.
        let shell = Shell::PowerShell7 {
            executable: PathBuf::from("unused-pwsh.exe"),
            version: "7.0.0".into(),
            bracket_cwd: result,
        };
        let fixture = crate::workspace_fixture::Fixture::new();
        fixture.write("child [1]/marker.txt", "CHILD");
        let workspace = Workspace::open(&fixture.0)
            .unwrap()
            .with_access(Access::Local);
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        };
        for _ in 0..2 {
            let proposal = prepare_with_shell(
                &workspace,
                "Write-Output safe",
                "child [1]",
                5,
                &shell,
                &budget,
            )
            .unwrap();
            assert!(
                proposal
                    .preview
                    .shell
                    .contains("supported by session probe")
            );
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn successful_exit_with_wrong_parent_content_is_incompatible() {
        let root = Cell::new(None);
        let result = check_with(&std::env::temp_dir(), None, |fixture| {
            root.set(Some(fixture.root.clone()));
            let stdout = synthetic_success(fixture).replace("parent=PARENT", "parent=CHILD");
            Ok(outcome(Stop::Exited, stdout))
        });
        assert_eq!(result, BracketCwd::Incompatible);
        assert!(!root.take().unwrap().exists(), "fixture must be removed");
        assert_blocks_user_script(result, "failed the bracketed-directory capability probe");
    }

    #[test]
    fn timeout_and_malformed_output_are_inconclusive_and_cleaned() {
        for (outcome, expected, diagnostic) in [
            (
                outcome(Stop::Timeout, String::new()),
                ProbeIssue::Timeout,
                "probe timed out",
            ),
            (
                outcome(Stop::Exited, "garbage".into()),
                ProbeIssue::Malformed,
                "probe returned malformed output",
            ),
        ] {
            let root = Cell::new(None);
            let result = check_with(&std::env::temp_dir(), None, |fixture| {
                root.set(Some(fixture.root.clone()));
                Ok(outcome)
            });
            assert_eq!(result, BracketCwd::Inconclusive(expected));
            assert!(!root.take().unwrap().exists(), "fixture must be removed");
            assert_blocks_user_script(result, diagnostic);
        }
    }

    #[test]
    fn fixture_failure_never_runs_the_probe_or_a_user_command() {
        let mut outer = Fixture::create(&std::env::temp_dir(), None).unwrap();
        let blocked_base = outer.root.join("ordinary-file.txt");
        fs::write(&blocked_base, "file, not a directory").unwrap();
        let result = check_with(&blocked_base, None, |_| panic!("must not execute"));
        assert_eq!(result, BracketCwd::Inconclusive(ProbeIssue::Fixture));
        assert_blocks_user_script(result, "temporary fixture");
        let result = check_with(&outer.root, Some(&outer.root), |_| {
            panic!("must never probe inside the selected directory")
        });
        assert_eq!(result, BracketCwd::Inconclusive(ProbeIssue::Fixture));
        outer.cleanup().unwrap();
    }

    #[test]
    fn probe_launch_failure_is_inconclusive_and_cleans_fixture() {
        let root = Cell::new(None);
        let result = check_with(&std::env::temp_dir(), None, |fixture| {
            root.set(Some(fixture.root.clone()));
            Err(ProbeIssue::Launch)
        });
        assert_eq!(result, BracketCwd::Inconclusive(ProbeIssue::Launch));
        assert!(!root.take().unwrap().exists(), "fixture must be removed");
        assert_blocks_user_script(result, "probe could not start or finish");
    }
}
