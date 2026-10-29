use super::*;

// Wrap the real bridge's write calls. A complete target reply is visible
// before the pending PTY input receives an injected kernel EIO.
const HARNESS: &str = r#"
#define _XOPEN_SOURCE 600
#include <sys/types.h>
#include <unistd.h>
static int status_gate[2] = { -1, -1 };
static ssize_t probe_write(int fd, const void *bytes, size_t length);
#define write probe_write
#define main bridge_entry
#include "unix.c"
#undef main
#undef write

static int saw_end;
static int injected;

static ssize_t probe_write(int fd, const void *bytes, size_t length) {
    const char *mode = getenv("JECODE_PTY_TEST_INJECT");
    const unsigned char *cursor = bytes;
    if (fd == STDOUT_FILENO) {
        static size_t matched;
        const char marker[] = ":END";
        for (size_t index = 0; index < length; ++index) {
            matched = cursor[index] == marker[matched]
                ? matched + 1 : cursor[index] == ':' ? 1 : 0;
            if (matched == sizeof(marker) - 1) {
                saw_end = 1;
                matched = 0;
            }
        }
    }
    if (mode && length == sizeof(int) && fd > STDERR_FILENO && !isatty(fd)) {
        struct pollfd gate = { status_gate[0], POLLIN, 0 };
        unsigned char marker;
        if (poll(&gate, 1, 1000) != 1 ||
            read(status_gate[0], &marker, 1) != 1 || marker != 1) {
            fprintf(stderr, "PROBE_GATE_TIMEOUT\n");
            int failed = -1;
            return write(fd, &failed, sizeof(failed));
        }
        if (strcmp(mode, "eio_cleanup") == 0) {
            int failed = -1;
            fprintf(stderr, "PROBE_CLEANUP_FAILURE\n");
            return write(fd, &failed, sizeof(failed));
        }
    }
    if (mode && !injected && length >= 100 && cursor[0] == 'x' &&
        fd > STDERR_FILENO) {
        if (!saw_end) {
            errno = EAGAIN;
            return -1;
        }
        injected = 1;
        unsigned char marker = 1;
        if (write(status_gate[1], &marker, 1) != 1) {
            fprintf(stderr, "PROBE_GATE_SIGNAL_FAILED\n");
            errno = EIO;
            return -1;
        }
        int injected_errno = strcmp(mode, "other_error") == 0 ? EINVAL : EIO;
        fprintf(stderr, "PROBE_MASTER_WRITE_%s\n",
                injected_errno == EIO ? "EIO" : "EINVAL");
        errno = injected_errno;
        return -1;
    }
    return write(fd, bytes, length);
}

int main(int argc, char **argv) {
    if (pipe(status_gate) < 0 ||
        fcntl(status_gate[0], F_SETFD, FD_CLOEXEC) < 0 ||
        fcntl(status_gate[1], F_SETFD, FD_CLOEXEC) < 0) return 125;
    int result = bridge_entry(argc, argv);
    close(status_gate[0]);
    close(status_gate[1]);
    return result;
}
"#;

struct Probe {
    directory: PathBuf,
    executable: PathBuf,
}

impl Probe {
    fn compile() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "jecode-pty-eio-contract-{}",
            crate::sessions::identifier()
        ));
        fs::create_dir(&directory).unwrap();
        let executable = directory.join("bridge-probe");
        let mut compiler = Command::new("cc")
            .args(["-std=c11", "-O2", "-Wall", "-Wextra", "-Werror"])
            .arg("-I")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/pty"))
            .args(["-x", "c", "-", "-o"])
            .arg(&executable)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        compiler
            .stdin
            .take()
            .unwrap()
            .write_all(HARNESS.as_bytes())
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

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.executable);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn run(probe: &Probe, mode: &str, child_code: i32, expected: i32) {
    let directory = std::env::temp_dir().join(format!(
        "jecode-pty-eio-run-{}",
        crate::sessions::identifier()
    ));
    fs::create_dir(&directory).unwrap();
    let endpoint = directory.join("control");
    let script = format!(
        "stty -echo; trap 'kill \"$sleeper\" 2>/dev/null; wait \"$sleeper\" 2>/dev/null; printf INTERRUPTED\\n' INT; sleep 30 & sleeper=$!; printf 'READY\\n'; wait \"$sleeper\"; IFS= read -r line; printf 'LINE:%s:END\\n' \"$line\"; exit {child_code}"
    );
    let mut command = Command::new(&probe.executable);
    command
        .arg(&endpoint)
        .args(["80", "24", "/bin/sh", "-c", &script])
        .env("JECODE_PTY_TEST_INJECT", mode)
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut bridge = Bridge {
        child: command.spawn().unwrap(),
        directory,
    };
    let endpoint = endpoint.to_str().unwrap();
    let cancellation = Cancellation::default();
    let mut input = connect(endpoint, &cancellation).unwrap();
    let mut control = connect_control(endpoint, &cancellation).unwrap();
    let (receiver, reader) = output_events(&mut bridge);
    let mut seen = Vec::new();
    until(&receiver, &mut seen, b"READY");
    input.write_all(&input_frame(1, b"FUTURE\n")).unwrap();
    let packet = input_frame(1, &[b'x'; 4096]);
    let mut saturated = false;
    for _ in 0..512 {
        let mut remaining = packet.as_slice();
        while !remaining.is_empty() {
            match input.write(remaining) {
                Ok(count) => remaining = &remaining[count..],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    saturated = true;
                    break;
                }
                Err(error) => panic!("input socket failed: {error}"),
            }
        }
        if saturated {
            break;
        }
    }
    assert!(saturated, "input did not encounter backpressure");
    control.write_all(&interrupt_frame(1)).unwrap();
    until(&receiver, &mut seen, b":END");
    assert!(
        String::from_utf8_lossy(&seen).contains("INTERRUPTED"),
        "{seen:?}"
    );
    assert!(
        String::from_utf8_lossy(&seen).contains("LINE:FUTURE:END"),
        "{seen:?}"
    );
    let status = finished(&mut bridge.child);
    let mut diagnostics = String::new();
    bridge
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostics)
        .unwrap();
    reader.join().unwrap();
    assert_eq!(status.code(), Some(expected), "{diagnostics}");
    assert!(
        diagnostics.contains(if mode == "other_error" {
            "PROBE_MASTER_WRITE_EINVAL"
        } else {
            "PROBE_MASTER_WRITE_EIO"
        }),
        "{diagnostics}"
    );
    if mode == "eio_cleanup" {
        assert!(
            diagnostics.contains("Could not confirm Unix PTY session cleanup"),
            "{diagnostics}"
        );
    } else if mode == "other_error" {
        assert!(
            diagnostics.contains("Unix PTY master write failed"),
            "{diagnostics}"
        );
    } else {
        assert!(
            !diagnostics.contains("Unix PTY master write failed"),
            "{diagnostics}"
        );
    }
    assert!(!bridge.directory.join("control").exists());
}

#[test]
fn closed_pty_input_waits_for_authoritative_child_and_cleanup_status() {
    let probe = Probe::compile();
    run(&probe, "eio", 0, 0);
    run(&probe, "eio", 7, 7);
    run(&probe, "eio_cleanup", 0, 125);
    run(&probe, "other_error", 0, 125);
}
