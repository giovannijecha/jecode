use super::*;

// Wrap the real bridge's write calls. A complete target reply is visible
// before the pending PTY input receives an injected kernel EIO.
const HARNESS: &str = r#"
#define _XOPEN_SOURCE 600
#include <stdint.h>
#include <signal.h>
#include <sys/ioctl.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
static int status_gate[2] = { -1, -1 };
static ssize_t probe_write(int fd, const void *bytes, size_t length);
static int probe_waitid(idtype_t type, id_t id, siginfo_t *info, int options);
static int probe_ioctl(int fd, unsigned long request, uintptr_t argument);
#define write probe_write
#define waitid probe_waitid
#define ioctl(fd, request, argument) probe_ioctl(fd, request, (uintptr_t)(argument))
#define main bridge_entry
#include "unix.c"
#undef main
#undef write
#undef waitid
#undef ioctl

static int exit_resize_mode(void) {
    const char *mode = getenv("JECODE_PTY_TEST_INJECT");
    return mode && (!strcmp(mode, "exit_resize") ||
                    !strcmp(mode, "exit_resize_error") ||
                    !strcmp(mode, "exit_resize_epipe"));
}

/* A bounded, test-only event channel to the Rust fixture. S is held until the
   fixture releases watcher status; E and R are observations. */
static int probe_event(unsigned char kind, unsigned char result, int error,
                       int hold) {
    const char *path = getenv("JECODE_PTY_STATUS_GATE");
    if (!path) return -1;
    int socket_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (socket_fd < 0) return -1;
    struct sockaddr_un endpoint;
    memset(&endpoint, 0, sizeof(endpoint));
    endpoint.sun_family = AF_UNIX;
    if (strlen(path) >= sizeof(endpoint.sun_path)) {
        close(socket_fd);
        return -1;
    }
    strcpy(endpoint.sun_path, path);
    struct timeval limit = { 5, 0 };
    if (setsockopt(socket_fd, SOL_SOCKET, SO_RCVTIMEO, &limit, sizeof(limit)) < 0 ||
        connect(socket_fd, (struct sockaddr *)&endpoint, sizeof(endpoint)) < 0) {
        close(socket_fd);
        return -1;
    }
    unsigned char event[3] = { kind, result, (unsigned char)error };
    int sent = send(socket_fd, event, sizeof(event), 0) == (ssize_t)sizeof(event);
    if (sent && hold) {
        unsigned char release;
        sent = recv(socket_fd, &release, 1, 0) == 1 && release == 'g';
    }
    close(socket_fd);
    return sent ? 0 : -1;
}

static int probe_waitid(idtype_t type, id_t id, siginfo_t *info, int options) {
    int result = waitid(type, id, info, options);
    int saved_errno = errno;
    if (exit_resize_mode() && result == 0 && info && info->si_pid != 0 &&
        probe_event('E', 0, 0, 0) < 0)
        fprintf(stderr, "PROBE_EXIT_EVENT_FAILED\n");
    errno = saved_errno;
    return result;
}

static int probe_ioctl(int fd, unsigned long request, uintptr_t argument) {
    const char *mode = getenv("JECODE_PTY_TEST_INJECT");
    struct winsize *size = (struct winsize *)argument;
    int forced_failure = mode && (!strcmp(mode, "live_resize_error") ||
                                  !strcmp(mode, "exit_resize_error") ||
                                  !strcmp(mode, "exit_resize_epipe")) &&
        request == TIOCSWINSZ && size && size->ws_col == 81 && size->ws_row == 25;
    int result;
    if (forced_failure) {
        errno = EBADF;
        result = -1;
    } else {
        result = ioctl(fd, request, (void *)argument);
    }
    int saved_errno = errno;
    if (exit_resize_mode() && request == TIOCSWINSZ && argument) {
        if (size->ws_col == 81 && size->ws_row == 25 &&
            probe_event('R', result < 0, result < 0 ? saved_errno : 0, 0) < 0)
            _exit(125);
    }
    errno = saved_errno;
    return result;
}

static int saw_end;
static int injected;
static int status_gate_opened;

static int eio_mode(const char *mode) {
    return mode && (!strcmp(mode, "eio") || !strcmp(mode, "eio_cleanup") ||
                    !strcmp(mode, "other_error"));
}

static ssize_t probe_write(int fd, const void *bytes, size_t length) {
    const char *mode = getenv("JECODE_PTY_TEST_INJECT");
    const unsigned char *cursor = bytes;
    if (mode && !strcmp(mode, "exit_resize_epipe") && length == 1 &&
        cursor[0] == WATCHER_QUERY && fd > STDERR_FILENO && !isatty(fd)) {
        errno = EPIPE;
        return -1;
    }
    if (exit_resize_mode() && length == sizeof(int) && fd > STDERR_FILENO &&
        !isatty(fd)) {
        int status;
        memcpy(&status, bytes, sizeof(status));
        unsigned char phase = status == WATCHER_EXITED ? 'M' : 'S';
        if (probe_event(phase, status == -1, 0, 1) < 0) _exit(125);
    }
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
    if (eio_mode(mode) && length == sizeof(int) && fd > STDERR_FILENO &&
        !isatty(fd)) {
        int status;
        memcpy(&status, bytes, sizeof(status));
        if (status == WATCHER_EXITED && !status_gate_opened) {
            struct pollfd gate = { status_gate[0], POLLIN, 0 };
            unsigned char marker;
            if (poll(&gate, 1, 1000) != 1 ||
                read(status_gate[0], &marker, 1) != 1 || marker != 1) {
                fprintf(stderr, "PROBE_GATE_TIMEOUT\n");
                int failed = -1;
                return write(fd, &failed, sizeof(failed));
            }
            status_gate_opened = 1;
        }
        if (status != WATCHER_EXITED && strcmp(mode, "eio_cleanup") == 0) {
            int failed = -1;
            fprintf(stderr, "PROBE_CLEANUP_FAILURE\n");
            return write(fd, &failed, sizeof(failed));
        }
    }
    if (eio_mode(mode) && !injected && length >= 100 && cursor[0] == 'x' &&
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

static int probe_frame_errors(void) {
    struct frame frame = { .have = 0, .need = 5 };
    struct input_queue queue = {0};
    struct interrupt_state interrupt = {0};
    errno = 0;
    int read_result = read_frame(-1, &frame, -1, -1,
                                 NULL, &interrupt, &queue);
    int read_errno = errno;
    report_control_error(read_result, read_errno);

    int sockets[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) < 0) return 1;
    unsigned char resize[] = {2, 4, 0, 0, 0, 80, 0, 24, 0};
    if (write_all(sockets[1], resize, sizeof(resize)) < 0) return 2;
    frame.have = 0;
    frame.need = 5;
    int resize_result = read_frame(sockets[0], &frame, -1, -1,
                                   NULL, &interrupt, &queue);
    if (resize_result == 0)
        resize_result = read_frame(sockets[0], &frame, -1, -1,
                                   NULL, &interrupt, &queue);
    int resize_errno = errno;
    report_control_error(resize_result, resize_errno);

    unsigned char invalid[] = {9, 0, 0, 0, 0};
    if (write_all(sockets[1], invalid, sizeof(invalid)) < 0) return 3;
    frame.have = 0;
    frame.need = 5;
    int invalid_result = read_frame(sockets[0], &frame, -1, -1,
                                    NULL, &interrupt, &queue);
    report_control_error(invalid_result, errno);
    close(sockets[1]);
    frame.have = 0;
    frame.need = 5;
    int closed_result = read_frame(sockets[0], &frame, -1, -1,
                                   NULL, &interrupt, &queue);
    report_control_error(closed_result, errno);
    close(sockets[0]);
    return read_result == FRAME_READ_FAILED && read_errno == EBADF &&
        resize_result == FRAME_RESIZE_FAILED && resize_errno == EBADF &&
        invalid_result == FRAME_INVALID && closed_result == FRAME_CLOSED ? 0 : 4;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "frame-errors") == 0)
        return probe_frame_errors();
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

#[test]
fn control_errors_identify_the_failed_operation() {
    let probe = Probe::compile();
    let output = Command::new(&probe.executable)
        .arg("frame-errors")
        .output()
        .unwrap();
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{diagnostics}");
    assert_eq!(
        diagnostics,
        "Unix PTY control read failed: Bad file descriptor\n\
         Unix PTY resize failed: Bad file descriptor\n\
         Unix PTY control frame was invalid\n\
         Unix PTY control channel closed\n"
    );
}

#[path = "unix_exit_resize_contract_tests.rs"]
mod exit_resize;
