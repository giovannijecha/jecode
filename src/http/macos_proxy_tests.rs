use std::process::{Command, Stdio};

#[test]
fn macos_proxy_routes_auth_and_socks_use_native_adapter() {
    let home = crate::test_support::Directory::new();
    let mut child = Command::new(concat!(env!("OUT_DIR"), "/http-proxy-fixture"))
        .env_clear()
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let output = child.wait_with_output().unwrap();
    drop(stdin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "fixture failed: {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("http-proxy-fixture: passed\n"),
        "missing completion marker: {stdout}\n{stderr}"
    );
}
