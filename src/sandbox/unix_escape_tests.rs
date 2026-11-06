//! Routes around the Workspace policy on Linux and macOS, each through the tool
//! path a model uses. Terminal access is covered by `native_unix_*` in the TUI tests,
//! which run under a real controlling terminal.

use super::unix_tests::{assert_exit, available, exec, fixture, run, text};
use super::*;
use crate::test_support::Directory;

/// The dynamic loader acts before `main`, so these would run code in the guardian
/// outside the policy. Its trace names each program it starts, and the guardian must
/// not be one of them; the command still gets the value it asked for.
#[test]
fn loader_variables_reach_the_command_but_never_the_guardian() {
    if !available() {
        return;
    }
    let root = Directory::new();
    let tools = fixture(root.path());
    let trace = root.path().join("trace");
    let result = run(
        &tools,
        "exec",
        Value::object([
            ("executable", Value::string("/bin/sh")),
            (
                "args",
                Value::Array(vec![
                    Value::string("-c"),
                    Value::string("printf '[%s]' \"$LD_DEBUG_OUTPUT\""),
                ]),
            ),
            (
                "env",
                Value::object([
                    ("LD_DEBUG", Value::string("files")),
                    ("LD_DEBUG_OUTPUT", Value::string(trace.to_string_lossy())),
                    ("DYLD_PRINT_LIBRARIES", Value::string("1")),
                ]),
            ),
            ("timeout_seconds", Value::number(30)),
        ]),
    );
    assert_exit(&result, 0);
    assert!(!text(&result, "stderr").contains("dyld"), "{result:?}");
    #[cfg(target_os = "linux")]
    {
        assert_eq!(
            text(&result, "stdout"),
            format!("[{}]", trace.display()),
            "{result:?}"
        );
        for entry in std::fs::read_dir(root.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.to_string_lossy().contains("trace.") {
                let contents = std::fs::read_to_string(&path).unwrap_or_default();
                assert!(!contents.contains("process_unix"), "{contents}");
            }
        }
    }
}

/// A Workspace command must keep working when the home folder does not exist.
#[test]
fn commands_run_without_a_home_folder() {
    const MISSING: &str = "/nonexistent/jecode-home";
    if !available() {
        return;
    }
    if std::env::var_os("HOME").is_none_or(|home| home != MISSING) {
        let name = "sandbox::unix_escape_tests::commands_run_without_a_home_folder";
        let status = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--nocapture"])
            .env("HOME", MISSING)
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let root = Directory::new();
    let tools = fixture(root.path());
    assert_exit(&exec(&tools, "/bin/sh", &["-c", "exit 0"]), 0);
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::cancel::Cancellation;
    use std::time::Duration;

    #[test]
    fn links_and_clones_cannot_carry_protected_contents_into_the_folder() {
        if !available() {
            return;
        }
        let root = Directory::new();
        let outside = Directory::new();
        let private = outside.path().join("private.txt");
        std::fs::write(&private, "private").unwrap();
        let tools = fixture(root.path());
        let script = format!(
            "ln '{0}' linked; cp -c '{0}' cloned; printf changed >> linked; cat linked cloned",
            private.display()
        );
        let result = exec(&tools, "/bin/sh", &["-c", &script]);
        assert!(!text(&result, "stdout").contains("private"), "{result:?}");
        assert!(
            !std::fs::read(root.path().join("cloned")).is_ok_and(|bytes| bytes == b"private"),
            "{result:?}"
        );
        assert_eq!(std::fs::read_to_string(&private).unwrap(), "private");
        // Package managers and compilers do both inside the folder.
        let inside = exec(
            &tools,
            "/bin/sh",
            &[
                "-c",
                "printf own > a && ln a b && cp -c a c && cp -c /etc/hosts d && cat b c",
            ],
        );
        assert_exit(&inside, 0);
        assert_eq!(text(&inside, "stdout"), "ownown");
    }

    /// A hard link would give a read-only file a writable name inside the folder.
    #[test]
    fn a_read_only_grant_cannot_be_linked_into_the_folder_and_changed() {
        if !available() {
            return;
        }
        let root = Directory::new();
        let runtime = Directory::new();
        let installed = runtime.path().join("installed");
        std::fs::write(&installed, "original").unwrap();
        let mut policy = Policy::for_command(Level::Workspace, root.path(), None, None)
            .unwrap()
            .unwrap();
        policy
            .allow_existing_read_directory(runtime.path(), None)
            .unwrap();
        let script = format!(
            "cat '{0}' && ln '{0}' linked; printf changed >> linked",
            installed.display()
        );
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &script]).current_dir(root.path());
        apply(&mut command, Some(&policy)).unwrap();
        let output = crate::process::run_leaf(
            &mut command,
            None,
            Some(Duration::from_secs(30)),
            4096,
            &Cancellation::default(),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_eq!(output.stdout, b"original", "the read grant must work");
        assert_eq!(std::fs::read_to_string(&installed).unwrap(), "original");
    }

    /// The canary is in this process's environment only after running itself again.
    #[test]
    fn another_process_keeps_its_environment_and_cannot_be_signalled() {
        const SECRET: &str = "JECODE_PS_CANARY";
        if !available() {
            return;
        }
        if std::env::var_os(SECRET).is_none() {
            let name = "sandbox::unix_escape_tests::macos::another_process_keeps_its_environment_and_cannot_be_signalled";
            let status = Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--nocapture"])
                .env(SECRET, "must-stay-out")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let pid = std::process::id();
        let script = format!("ps -Eww -o command= -p {pid}; kill -0 {pid} && echo signalled");
        let visible = Command::new("/bin/sh")
            .args(["-c", &script])
            .output()
            .unwrap();
        // Unconfined, the same command proves the probe can see both.
        assert!(String::from_utf8_lossy(&visible.stdout).contains("signalled"));
        let root = Directory::new();
        let tools = fixture(root.path());
        let result = exec(&tools, "/bin/sh", &["-c", &script]);
        let stdout = text(&result, "stdout");
        assert!(!stdout.contains("must-stay-out"), "{result:?}");
        assert!(!stdout.contains("signalled"), "{result:?}");
    }

    #[test]
    fn a_copied_launchctl_cannot_start_work_outside_the_policy() {
        if !available() {
            return;
        }
        let root = Directory::new();
        let outside = Directory::new();
        let escaped = outside.path().join("escaped");
        let label = format!("dev.jecode.test.{}", std::process::id());
        let tools = fixture(root.path());
        let script = format!(
            "cp /bin/launchctl ./l && ./l submit -l {label} -- /usr/bin/touch '{}'",
            escaped.display()
        );
        let result = exec(&tools, "/bin/sh", &["-c", &script]);
        std::thread::sleep(Duration::from_secs(2));
        let _ = Command::new("/bin/launchctl")
            .args(["remove", &label])
            .status();
        assert!(!escaped.exists(), "{result:?}");
    }
}
