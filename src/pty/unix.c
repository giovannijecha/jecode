#define _XOPEN_SOURCE 600
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

/* Native POSIX boundary: a detached watcher owns the PTY child. The bridge
   stays in Jecode's ordinary supervised process group. */
static volatile sig_atomic_t stopping;

static void stop(int signal_number) {
    (void)signal_number;
    stopping = 1;
}

static int write_all(int fd, const void *bytes, size_t length) {
    const unsigned char *cursor = bytes;
    while (length) {
        ssize_t count = write(fd, cursor, length);
        if (count > 0) {
            cursor += count;
            length -= (size_t)count;
        } else if (count < 0 && errno == EINTR) {
            continue;
        } else {
            return -1;
        }
    }
    return 0;
}

static int set_nonblocking(int fd) {
    int flags = fcntl(fd, F_GETFL);
    return flags < 0 ? -1 : fcntl(fd, F_SETFL, flags | O_NONBLOCK);
}

static void kill_session(pid_t child) {
    /* The direct PID also covers the instant before the child calls setsid. */
    if (child > 0) {
        (void)kill(-child, SIGKILL);
        (void)kill(child, SIGKILL);
    }
}

/* POSIX has no signal-to-session operation. ps supplies candidate PIDs on
   Linux and macOS; getsid verifies each immediately before signaling it.
   The session leader remains unreaped, so its session ID cannot be reused. */
static int sweep_session(pid_t leader, int *active) {
    int output[2];
    if (pipe(output) < 0) return -1;
    pid_t probe = fork();
    if (probe < 0) {
        close(output[0]);
        close(output[1]);
        return -1;
    }
    if (probe == 0) {
        close(output[0]);
        if (dup2(output[1], STDOUT_FILENO) < 0) _exit(127);
        close(output[1]);
        execl("/bin/ps", "ps", "-A", "-o", "pid=", "-o", "stat=",
              (char *)NULL);
        _exit(127);
    }
    close(output[1]);
    FILE *lines = fdopen(output[0], "r");
    if (!lines) {
        close(output[0]);
        (void)kill(probe, SIGKILL);
        (void)waitpid(probe, NULL, 0);
        return -1;
    }
    char line[128], state[32];
    *active = 0;
    int failure = 0;
    while (fgets(line, sizeof(line), lines)) {
        long number;
        if (sscanf(line, "%ld %31s", &number, state) != 2 ||
            number <= 0 || number > INT_MAX || number == leader ||
            state[0] == 'Z') continue;
        pid_t pid = (pid_t)number;
        if (getsid(pid) == leader) {
            ++*active;
            if (kill(pid, SIGKILL) < 0 && errno != ESRCH) failure = 1;
        }
    }
    if (ferror(lines)) failure = 1;
    fclose(lines);
    int status;
    while (waitpid(probe, &status, 0) < 0) {
        if (errno != EINTR) return -1;
    }
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) failure = 1;
    return failure ? -1 : 0;
}

static int stop_session(pid_t leader) {
    kill_session(leader);
    for (int pass = 0; pass < 8; ++pass) {
        int active;
        if (sweep_session(leader, &active) < 0) return -1;
        if (!active) return 0;
        struct timespec pause = { 0, 20 * 1000 * 1000 };
        while (nanosleep(&pause, &pause) < 0 && errno == EINTR) {}
    }
    return -1;
}

static void target(int master, const char *slave_name, int owner_read,
                   int owner_write, int status_write, int control, char **argv) {
    close(master);
    close(owner_read);
    close(owner_write);
    close(status_write);
    close(control);
    if (setsid() < 0) _exit(126);
    signal(SIGINT, SIG_DFL);
    signal(SIGQUIT, SIG_DFL);
    signal(SIGTERM, SIG_DFL);
    signal(SIGHUP, SIG_DFL);
    signal(SIGPIPE, SIG_DFL);
    int slave = open(slave_name, O_RDWR);
    if (slave < 0 || ioctl(slave, TIOCSCTTY, 0) < 0) _exit(126);
    if (dup2(slave, STDIN_FILENO) < 0 || dup2(slave, STDOUT_FILENO) < 0 ||
        dup2(slave, STDERR_FILENO) < 0) _exit(126);
    if (slave > STDERR_FILENO) close(slave);
    execvp(argv[0], argv);
    _exit(errno == ENOENT ? 127 : 126);
}

static void watch_child(int master, const char *slave_name, int owner_read,
                        int owner_write, int status_read, int status_write,
                        int control, char **argv) {
    close(master);
    close(control);
    close(owner_write);
    close(status_read);
    /* The bridge may be SIGKILLed before this watcher finishes its session
       sweep. Keep its output pipe open until cleanup is complete, so Jecode's
       capture reader cannot report EOF (and a stopped receipt) too early. */
    int barrier = dup(STDOUT_FILENO);
    if (barrier < 0 || fcntl(barrier, F_SETFD, FD_CLOEXEC) < 0) _exit(125);
    int null = open("/dev/null", O_RDWR);
    if (null < 0 || dup2(null, STDIN_FILENO) < 0 ||
        dup2(null, STDOUT_FILENO) < 0 || dup2(null, STDERR_FILENO) < 0)
        _exit(125);
    if (null > STDERR_FILENO) close(null);
    if (setsid() < 0) _exit(125);
    pid_t child = fork();
    if (child < 0) _exit(125);
    if (child == 0)
        target(master, slave_name, owner_read, owner_write, status_write,
               control, argv);

    int status = 0;
    for (;;) {
        siginfo_t info;
        memset(&info, 0, sizeof(info));
        int observed = waitid(P_PID, (id_t)child, &info, WEXITED | WNOHANG | WNOWAIT);
        if (observed == 0 && info.si_pid == child) {
            /* Keep the leader's PID reserved until its whole group is killed. */
            int cleaned = stop_session(child);
            while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
            if (cleaned < 0) status = -1;
            (void)write_all(status_write, &status, sizeof(status));
            break;
        }
        if (observed < 0 && errno != EINTR) {
            (void)stop_session(child);
            break;
        }
        struct pollfd owner = { owner_read, POLLIN | POLLHUP, 0 };
        int ready = poll(&owner, 1, 50);
        if (ready > 0 && owner.revents) {
            unsigned char byte;
            ssize_t count = read(owner_read, &byte, 1);
            if (count == 0 || (count < 0 && errno != EINTR)) {
                (void)stop_session(child);
                while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
                break;
            }
        }
    }
    close(owner_read);
    close(status_write);
    close(barrier);
    _exit(0);
}

static int terminal_size(int master, uint16_t columns, uint16_t rows) {
    struct winsize size;
    memset(&size, 0, sizeof(size));
    size.ws_col = columns;
    size.ws_row = rows;
    return ioctl(master, TIOCSWINSZ, &size);
}

/* One socket frame: opcode, little-endian u32 length, payload. Input frames
   are bounded to 4096 bytes; resize carries two little-endian u16 values. */
struct frame {
    unsigned char bytes[5 + 4096];
    size_t have;
    size_t need;
    size_t pending;
    size_t sent;
};

static int read_frame(int control, struct frame *frame, int master) {
    ssize_t count = read(control, frame->bytes + frame->have,
                         frame->need - frame->have);
    if (count == 0) return -1;
    if (count < 0) return errno == EAGAIN || errno == EINTR ? 0 : -1;
    frame->have += (size_t)count;
    if (frame->have != frame->need) return 0;
    if (frame->need == 5) {
        uint32_t length = (uint32_t)frame->bytes[1] |
            ((uint32_t)frame->bytes[2] << 8) |
            ((uint32_t)frame->bytes[3] << 16) |
            ((uint32_t)frame->bytes[4] << 24);
        unsigned char operation = frame->bytes[0];
        if ((operation == 1 && length > 4096) ||
            (operation == 2 && length != 4) ||
            (operation == 3 && length != 0) ||
            (operation != 1 && operation != 2 && operation != 3)) return -2;
        frame->need += length;
        if (length) return 0;
    }
    if (frame->bytes[0] == 1) {
        frame->pending = frame->need - 5;
        frame->sent = 0;
    } else if (frame->bytes[0] == 2) {
        const unsigned char *payload = frame->bytes + 5;
        uint16_t columns = (uint16_t)(payload[0] | (payload[1] << 8));
        uint16_t rows = (uint16_t)(payload[2] | (payload[3] << 8));
        if (!columns || !rows || terminal_size(master, columns, rows) < 0)
            return -2;
    } else {
        frame->bytes[5] = 3;
        frame->pending = 1;
        frame->sent = 0;
    }
    if (!frame->pending) {
        frame->have = 0;
        frame->need = 5;
    }
    return 0;
}

static int bridge(int master, int control, int owner_write, int status_read) {
    struct frame frame;
    memset(&frame, 0, sizeof(frame));
    frame.need = 5;
    int child_status = 0;
    int child_done = 0;
    for (;;) {
        if (stopping) return 125;
        struct pollfd events[3] = {
            { master, POLLIN | (frame.pending ? POLLOUT : 0), 0 },
            { control, frame.pending ? 0 : POLLIN, 0 },
            { status_read, child_done ? 0 : POLLIN, 0 },
        };
        int ready = poll(events, 3, child_done ? 100 : 1000);
        if (ready < 0) {
            if (errno == EINTR) continue;
            return 125;
        }
        if (events[2].revents) {
            ssize_t count = read(status_read, &child_status, sizeof(child_status));
            if (count != (ssize_t)sizeof(child_status)) return 125;
            if (child_status == -1) {
                fprintf(stderr, "Could not confirm Unix PTY session cleanup\n");
                return 125;
            }
            child_done = 1;
        }
        if (events[0].revents & (POLLIN | POLLHUP | POLLERR)) {
            unsigned char output[8192];
            ssize_t count = read(master, output, sizeof(output));
            if (count > 0) {
                if (write_all(STDOUT_FILENO, output, (size_t)count) < 0) return 125;
            } else if (count == 0 || errno == EIO) {
                if (child_done) break;
            } else if (errno != EINTR && errno != EAGAIN) {
                return 125;
            }
        }
        if ((events[0].revents & POLLOUT) && frame.pending) {
            ssize_t count = write(master, frame.bytes + 5 + frame.sent,
                                  frame.pending);
            if (count > 0) {
                frame.pending -= (size_t)count;
                frame.sent += (size_t)count;
                if (!frame.pending) {
                    frame.have = 0;
                    frame.need = 5;
                }
            } else if (count < 0 && errno != EINTR && errno != EAGAIN) {
                return 125;
            }
        }
        if (events[1].revents) {
            if (read_frame(control, &frame, master) < 0) return 125;
        }
        if (child_done && ready == 0) break;
    }
    close(owner_write);
    if (WIFEXITED(child_status)) return WEXITSTATUS(child_status);
    if (WIFSIGNALED(child_status)) return 128 + WTERMSIG(child_status);
    return 125;
}

int main(int argc, char **argv) {
    if (argc < 5) return 125;
    char *end;
    long columns = strtol(argv[2], &end, 10);
    if (*end || columns < 1 || columns > 65535) return 125;
    long rows = strtol(argv[3], &end, 10);
    if (*end || rows < 1 || rows > 65535) return 125;
    struct sockaddr_un endpoint;
    memset(&endpoint, 0, sizeof(endpoint));
    endpoint.sun_family = AF_UNIX;
    if (strlen(argv[1]) >= sizeof(endpoint.sun_path)) {
        fprintf(stderr, "Unix PTY endpoint path is too long\n");
        return 125;
    }
    strcpy(endpoint.sun_path, argv[1]);
    int listener = socket(AF_UNIX, SOCK_STREAM, 0);
    if (listener < 0 || bind(listener, (struct sockaddr *)&endpoint,
                             sizeof(endpoint)) < 0 || chmod(argv[1], 0600) < 0 ||
        listen(listener, 1) < 0) {
        fprintf(stderr, "Could not listen on Unix PTY endpoint: %s\n",
                strerror(errno));
        return 125;
    }
    int control = accept(listener, NULL, NULL);
    close(listener);
    if (control < 0) return 125;
    int master = posix_openpt(O_RDWR | O_NOCTTY | O_NONBLOCK);
    if (control < 0 || master < 0 || grantpt(master) < 0 || unlockpt(master) < 0 ||
        terminal_size(master, (uint16_t)columns, (uint16_t)rows) < 0) {
        fprintf(stderr, "Could not initialize Unix PTY: %s\n", strerror(errno));
        return 125;
    }
    char *slave_name = ptsname(master);
    if (!slave_name) return 125;
    int owner[2], status[2];
    if (pipe(owner) < 0 || pipe(status) < 0) return 125;
    pid_t watcher = fork();
    if (watcher < 0) return 125;
    if (watcher == 0)
        watch_child(master, slave_name, owner[0], owner[1], status[0], status[1],
                    control, argv + 4);
    close(owner[0]);
    close(status[1]);
    if (set_nonblocking(control) < 0) return 125;
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = stop;
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    sigaction(SIGINT, &action, NULL);
    sigaction(SIGHUP, &action, NULL);
    signal(SIGPIPE, SIG_IGN);
    int result = bridge(master, control, owner[1], status[0]);
    close(owner[1]);
    close(status[0]);
    close(control);
    close(master);
    unlink(argv[1]);
    return result;
}
