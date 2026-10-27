use super::*;

// Include the owned bridge so this checks its real static interrupt functions
// against a PTY that has not acquired a controlling session yet.
const HARNESS: &str = r#"
#define main bridge_entry
#include "unix.c"
#undef main

int main(void) {
    alarm(5);
    int master = posix_openpt(O_RDWR | O_NOCTTY | O_NONBLOCK);
    if (master < 0) { perror("posix_openpt"); return 1; }
    if (grantpt(master) < 0 || unlockpt(master) < 0) {
        perror("grantpt/unlockpt"); return 1;
    }
    char *name = ptsname(master);
    if (!name) { perror("ptsname"); return 1; }
    int slave = open(name, O_RDWR | O_NOCTTY);
    if (slave < 0) { perror("open slave"); return 1; }
    pid_t foreground = tcgetpgrp(master);
    if (foreground != 0 &&
        !(foreground < 0 && (errno == ENOTTY || errno == EIO))) {
        fprintf(stderr, "unexpected foreground: %ld, errno %d\n",
                (long)foreground, errno);
        return 1;
    }
    int flush = -1;
    if (send_interrupt(master, slave, &flush) != 1 || flush != 0) {
        fprintf(stderr, "no-foreground interrupt remained pending\n");
        return 1;
    }
    struct input_queue queue;
    struct interrupt_state state;
    memset(&queue, 0, sizeof(queue));
    memset(&state, 0, sizeof(state));
    state.count = 1;
    state.generation[0] = 1;
    if (deliver_interrupt(master, slave, &queue, &state) != 1 ||
        state.count != 0 || state.applied_generation != 1) {
        fprintf(stderr, "no-foreground interrupt blocked the queue\n");
        return 1;
    }
    const char message[] = "hello\n";
    if (write(master, message, sizeof(message) - 1) !=
        (ssize_t)(sizeof(message) - 1)) {
        perror("write master"); return 1;
    }
    char received[sizeof(message)];
    ssize_t count = read(slave, received, sizeof(received));
    if (count != (ssize_t)(sizeof(message) - 1) ||
        memcmp(received, message, sizeof(message) - 1) != 0) {
        fprintf(stderr, "input did not cross the PTY after interrupt\n");
        return 1;
    }
    close(slave);
    close(master);
    return 0;
}
"#;

struct TestBinary {
    directory: PathBuf,
    executable: PathBuf,
}

impl Drop for TestBinary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.executable);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[test]
fn interrupt_without_foreground_is_consumed_and_pty_stays_usable() {
    let directory = std::env::temp_dir().join(format!(
        "jecode-pty-contract-{}",
        crate::sessions::identifier()
    ));
    fs::create_dir(&directory).unwrap();
    let fixture = TestBinary {
        executable: directory.join("no-foreground"),
        directory,
    };
    let mut compiler = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/pty"))
        .args(["-x", "c", "-", "-o"])
        .arg(&fixture.executable)
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
    let compiled = compiler.wait_with_output().unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = Command::new(&fixture.executable).output().unwrap();
    assert!(
        run.status.success(),
        "status={:?}, stderr={}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
}
