use super::*;

const DETACHED_TARGET: &str = r#"
#define _XOPEN_SOURCE 600
#include <signal.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <unistd.h>

int main(void) {
    char line[64];
    struct winsize size;
    if (signal(SIGHUP, SIG_IGN) == SIG_ERR ||
        ioctl(STDIN_FILENO, TIOCNOTTY) < 0) return 2;
    puts("DETACHED");
    fflush(stdout);
    if (!fgets(line, sizeof(line), stdin) ||
        ioctl(STDIN_FILENO, TIOCGWINSZ, &size) < 0) return 3;
    printf("LINE:%sSIZE:%u:%u\n", line, size.ws_col, size.ws_row);
    return 0;
}
"#;

struct Fixture {
    directory: PathBuf,
    executable: PathBuf,
}

impl Fixture {
    fn compile() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "jecode-pty-fixture-{}",
            crate::sessions::identifier()
        ));
        fs::create_dir(&directory).unwrap();
        let executable = directory.join("detached-target");
        let mut compiler = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-x", "c", "-"])
            .arg("-o")
            .arg(&executable)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        compiler
            .stdin
            .take()
            .unwrap()
            .write_all(DETACHED_TARGET.as_bytes())
            .unwrap();
        let output = compiler.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            directory,
            executable,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.executable);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[test]
fn detached_target_reads_input_without_interrupt() {
    let fixture = Fixture::compile();
    let (mut bridge, mut input, _control) =
        launch_command(&[fixture.executable.to_str().unwrap()], None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"DETACHED");
    input.write_all(&input_frame(0, b"hello\n")).unwrap();
    until(&receiver, &mut seen, b"LINE:hello");
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}

#[test]
fn interrupts_without_foreground_do_not_block_input_or_resize() {
    let fixture = Fixture::compile();
    let (mut bridge, mut input, mut control) =
        launch_command(&[fixture.executable.to_str().unwrap()], None);
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"DETACHED");
    for generation in 1..=20 {
        control.write_all(&interrupt_frame(generation)).unwrap();
    }
    control
        .write_all(&crate::pty::frame(2, &[107, 0, 41, 0]))
        .unwrap();
    control.write_all(&interrupt_frame(21)).unwrap();
    input.write_all(&input_frame(21, b"hello\n")).unwrap();
    until(&receiver, &mut seen, b"SIZE:107:41");
    assert!(String::from_utf8_lossy(&seen).contains("LINE:hello\r\n"));
    assert_eq!(finished(&mut bridge.child).code(), Some(0));
    reader.join().unwrap();
}
