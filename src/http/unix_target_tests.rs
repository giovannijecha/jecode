#[test]
fn a_dns_name_starting_with_127_does_not_bypass_the_proxy() {
    let home = crate::test_support::Directory::new();
    let output = std::process::Command::new(concat!(env!("OUT_DIR"), "/http-target-fixture"))
        .env_clear()
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_tls_control_record_does_not_block_or_end_the_websocket() {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let home = crate::test_support::Directory::new();
    let mut child = Command::new(concat!(env!("OUT_DIR"), "/http-target-fixture"))
        .arg("late-ticket-input")
        .env_clear()
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut marker = [0];
    stdout.read_exact(&mut marker).unwrap();
    assert_eq!(&marker, b"T");
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"1\nx").unwrap();
    let mut frames = Vec::new();
    stdout.read_to_end(&mut frames).unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(0));
    assert_eq!(frames, b"D\x02\0\0\0ok");
}

#[test]
fn a_tls_peek_want_write_retries_when_the_socket_is_writable() {
    use std::process::{Command, Stdio};
    let home = crate::test_support::Directory::new();
    let mut child = Command::new(concat!(env!("OUT_DIR"), "/http-target-fixture"))
        .arg("late-ticket-write")
        .env_clear()
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let output = child.wait_with_output().unwrap();
    drop(stdin);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"D\x02\0\0\0ok");
}
