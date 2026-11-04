use super::*;
use crate::cancel::Cancellation;
use crate::test_support::{Directory, sandbox as probe};
use std::time::Duration;

#[test]
fn workspace_lists_drive_roots_only_for_installed_node() {
    let root = Directory::new();
    let outside = Directory::new();
    let runtime = Directory::new();
    let allowed = root.path().join("allowed.txt");
    let private = outside.path().join("private.txt");
    let escaped = outside.path().join("escaped.txt");
    std::fs::write(&allowed, "allowed").unwrap();
    std::fs::write(&private, "private").unwrap();
    std::fs::write(runtime.path().join("node.exe"), "").unwrap();
    let fixture = probe::executable(root.path());
    let run = |node: bool| {
        let mut policy = Policy::for_command(Level::Workspace, root.path(), None, None)
            .unwrap()
            .unwrap();
        if node {
            let search = runtime.path().as_os_str().to_os_string();
            policy
                .allow_node_tools_from(|name| (name == "PATH").then(|| search.clone()), None)
                .unwrap();
        }
        let mut command = Command::new(&fixture);
        command
            .current_dir(root.path())
            .env(probe::ROLE, "listing")
            .env(probe::ALLOWED, &allowed)
            .env(probe::PRIVATE, &private)
            .env(probe::ESCAPED, &escaped);
        apply(&mut command, Some(&policy)).unwrap();
        let output = crate::process::run_observed(
            &mut command,
            None,
            Duration::from_secs(5),
            1 << 16,
            &Cancellation::default(),
            None,
            &mut |_| Ok(()),
        )
        .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert_eq!(output.exit_code, Some(0), "{stdout}");
        stdout
    };
    // Node.js stats the drive root while resolving every script path. Only
    // an installed runtime lists drive roots; contents and writes stay denied.
    let confined = run(false);
    for expected in [
        "ROOT:PermissionDenied",
        "LIST:PermissionDenied",
        "READ:PermissionDenied",
        "WRITE:PermissionDenied",
    ] {
        assert!(confined.contains(expected), "{confined}");
    }
    let node = run(true);
    for expected in [
        "ROOT:ok",
        "LIST:ok",
        "READ:PermissionDenied",
        "WRITE:PermissionDenied",
    ] {
        assert!(node.contains(expected), "{node}");
    }
    assert!(!escaped.exists());
}
