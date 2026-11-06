//! Native macOS proof that a confined Cargo build does not consult xcrun.

use super::super::unix_tests::{assert_exit, available, fixture, run, text};
use crate::json::Value;
use crate::test_support::Directory;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn cargo_uses_the_discovered_sdk_without_xcrun() {
    const CHILD: &str = "JECODE_SDK_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // The test runner can inherit SDKROOT. Isolate this test's parent
        // environment without changing other concurrent tests.
        let selector = concat!(
            module_path!(),
            "::cargo_uses_the_discovered_sdk_without_xcrun"
        );
        let selector = selector
            .strip_prefix(concat!(env!("CARGO_PKG_NAME"), "::"))
            .unwrap_or(selector);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([selector, "--exact", "--nocapture"])
            .env_remove("SDKROOT")
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("running 1 test"),
            "{stdout}\n{stderr}"
        );
        return;
    }
    if !available() {
        return;
    }
    let root = Directory::new();
    let home = Directory::new();
    let blocker = root.path().join("blocker");
    std::fs::create_dir(&blocker).unwrap();
    let marker = root.path().join("xcrun-called");
    let xcrun = blocker.join("xcrun");
    std::fs::write(
        &xcrun,
        "#!/bin/sh\nprintf called > \"$JECODE_XCRUN_MARKER\"\nexit 91\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&xcrun, std::fs::Permissions::from_mode(0o755)).unwrap();
    let inherited_path =
        std::env::split_paths(&std::env::var_os("PATH").unwrap()).collect::<Vec<_>>();
    let path = std::env::join_paths(
        std::iter::once(blocker.as_path()).chain(inherited_path.iter().map(PathBuf::as_path)),
    )
    .unwrap();
    let mut tools = fixture(root.path());
    tools.configure_temporary(
        crate::scratch::Area::new(home.path().join("tmp"), root.path().to_path_buf(), "1-1-0")
            .unwrap(),
    );
    let cargo = |name: &str, remove_sdk: bool| {
        let script = format!(
            "mkdir {name} && cd {name} && mkdir src && printf '[package]\\nname=\"{name}\"\\nversion=\"0.1.0\"\\nedition=\"2021\"\\n[workspace]\\n' > Cargo.toml \
             && printf 'fn main(){{println!(\"built\");}}' > src/main.rs && cargo build --quiet --offline && ./target/debug/{name}"
        );
        let mut env = vec![
            ("PATH", Value::string(path.to_string_lossy())),
            (
                "JECODE_XCRUN_MARKER",
                Value::string(marker.to_string_lossy()),
            ),
        ];
        if remove_sdk {
            env.push(("SDKROOT", Value::Null));
        }
        run(
            &tools,
            "exec",
            Value::object([
                ("executable", Value::string("/bin/sh")),
                (
                    "args",
                    Value::Array(vec![Value::string("-c"), Value::string(script)]),
                ),
                (
                    "env",
                    Value::Object(
                        env.into_iter()
                            .map(|(key, value)| (key.into(), value))
                            .collect(),
                    ),
                ),
                ("timeout_seconds", Value::number(30)),
            ]),
        )
    };
    // This control proves that Rust reaches xcrun when SDKROOT is removed.
    let control = cargo("sdk_control", true);
    assert!(marker.exists(), "xcrun was not reached: {control:?}");
    std::fs::remove_file(&marker).unwrap();

    let inferred = cargo("sdk_inferred", false);
    assert_exit(&inferred, 0);
    assert_eq!(text(&inferred, "stdout"), "built\n");
    assert!(
        !marker.exists(),
        "xcrun was reached despite SDKROOT: {inferred:?}"
    );
}
