use super::*;
use std::path::PathBuf;

#[test]
#[ignore = "requires a native PTY and a staged npm package"]
fn npm_launcher_native_tui() {
    let node = PathBuf::from(
        std::env::var_os("JECODE_NPM_NODE").expect("set JECODE_NPM_NODE to the absolute node path"),
    );
    let launcher = PathBuf::from(
        std::env::var_os("JECODE_NPM_LAUNCHER")
            .expect("set JECODE_NPM_LAUNCHER to the packed bin/jecode.cjs"),
    );
    assert!(node.is_absolute() && node.is_file());
    assert!(launcher.is_absolute() && launcher.is_file());
    let mut terminal = driver::Pty::new_external(&node, &launcher);
    terminal.wait("Ask anything");
    terminal.wait("No configured provider");
    terminal.send(b"npm draft");
    terminal.wait("npm draft");
    terminal.resize(40, 12);
    terminal.resize(80, 24);
    let cleared = terminal.mark();
    terminal.send(b"\x03");
    terminal.wait_after("Ask anything", cleared);
    terminal.send(b"kept after resize");
    terminal.wait("kept after resize");
    terminal.send(b"\x11");
    terminal.finish("jecode closed");
}
