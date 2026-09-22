#[path = "support/workspace.rs"]
mod support;
use jecode::workspace::{Budget, Error, Workspace, relative};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use support::Fixture;

fn budget(cancelled: &AtomicBool) -> Budget<'_> {
    Budget {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}

#[test]
fn lists_and_reads_only_regular_visible_workspace_entries() {
    let fixture = Fixture::new();
    fixture.write("src/main.rs", "fn main() {}\n// café 中文\r\n");
    fixture.write("README.md", "fixture");
    fixture.write(".env", "hidden");
    fixture.write("target/output.txt", "generated");
    fixture.write("auth.json", "private");
    let workspace = Workspace::open(&fixture.0).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    for _ in 0..2 {
        let result = workspace.list(".", &budget).unwrap();
        assert_eq!(
            result
                .entries
                .iter()
                .map(|e| (e.name.as_str(), e.directory))
                .collect::<Vec<_>>(),
            [("README.md", false), ("src", true)]
        );
        assert_eq!(result.omitted, 3);
        assert!(!result.truncated);
    }
    assert_eq!(
        workspace.list("src", &budget).unwrap().entries[0].name,
        "main.rs"
    );
    assert_eq!(
        workspace.read("src/main.rs", &budget).unwrap(),
        "fn main() {}\n// café 中文\r\n"
    );
    assert_eq!(workspace.read(".env", &budget), Err(Error::Excluded));
    assert_eq!(workspace.read("src", &budget), Err(Error::Unavailable));
    assert_eq!(workspace.read("missing", &budget), Err(Error::Unavailable));
}

#[test]
fn paths_reject_escape_ambiguity_and_private_names() {
    for path in [
        "",
        "/tmp/file",
        "../file",
        "src/../file",
        "src//file",
        "./file",
        "C:/file",
        r"src\file",
        "file:stream",
        "CON.txt",
        "lpt1",
        "trailing.",
        "trailing ",
        "file\0",
        "FILE~1",
    ] {
        assert!(relative(path).is_err(), "{path:?}");
    }
    for path in [
        ".git/config",
        "src/.env",
        "AUTH.json",
        "server.PEM",
        "node_modules/package",
        "target/foo",
    ] {
        assert_eq!(relative(path), Err(Error::Excluded));
    }
    assert_eq!(relative("src/中文.rs").unwrap(), "src/中文.rs");
}

#[test]
fn text_and_operation_limits_fail_without_partial_content() {
    let fixture = Fixture::new();
    fixture.write("binary", b"hello\0world");
    fixture.write("ansi", b"hello\x1b[2J");
    fixture.write("invalid", [0xff]);
    fixture.write("large", vec![b'x'; jecode::workspace::MAX_FILE_BYTES + 1]);
    let workspace = Workspace::open(&fixture.0).unwrap();
    let cancelled = AtomicBool::new(false);
    for path in ["binary", "ansi", "invalid"] {
        assert_eq!(workspace.read(path, &budget(&cancelled)), Err(Error::Text));
    }
    assert_eq!(
        workspace.read("large", &budget(&cancelled)),
        Err(Error::Size)
    );
    let expired = Budget {
        cancelled: &cancelled,
        deadline: Instant::now(),
    };
    assert_eq!(workspace.read("binary", &expired), Err(Error::Timeout));
    cancelled.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        workspace.read("binary", &budget(&cancelled)),
        Err(Error::Cancelled)
    );
    assert!(matches!(
        workspace.list(".", &budget(&cancelled)),
        Err(Error::Cancelled)
    ));
}

#[cfg(unix)]
#[test]
fn links_cannot_escape_and_root_handle_survives_replacement() {
    use std::{fs, os::unix::fs::symlink};
    let fixture = Fixture::new();
    fixture.write("root/owned.txt", "owned");
    fixture.write("outside/private.txt", "outside");
    symlink("../outside", fixture.0.join("root/link")).unwrap();
    symlink("../outside/private.txt", fixture.0.join("root/file-link")).unwrap();
    let workspace = Workspace::open(&fixture.0.join("root")).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = budget(&cancelled);
    assert_eq!(workspace.read("owned.txt", &budget).unwrap(), "owned");
    assert_eq!(
        workspace.read("link/private.txt", &budget),
        Err(Error::Unavailable)
    );
    assert_eq!(
        workspace.read("file-link", &budget),
        Err(Error::Unavailable)
    );
    assert_eq!(workspace.list(".", &budget).unwrap().omitted, 2);
    fs::rename(fixture.0.join("root"), fixture.0.join("renamed")).unwrap();
    symlink("outside", fixture.0.join("root")).unwrap();
    // Some host-backed filesystems (including DrvFS) refuse relative resolution
    // after a directory rename. Failure is acceptable; reading the replacement is not.
    let result = workspace.read("owned.txt", &budget);
    assert!(result == Ok("owned".into()) || result == Err(Error::Unavailable));
    assert_eq!(
        workspace.read("private.txt", &budget),
        Err(Error::Unavailable)
    );
}
