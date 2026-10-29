#define _XOPEN_SOURCE 600
#include <errno.h>
#ifdef __linux__
#include <dirent.h>
#endif
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

#include "unix_session.h"

static int terminal_size(int slave, uint16_t columns, uint16_t rows) {
    struct winsize size;
    memset(&size, 0, sizeof(size));
    size.ws_col = columns;
    size.ws_row = rows;
    return ioctl(slave, TIOCSWINSZ, &size);
}

static int send_interrupt(int master, int slave, int *flush_input) {
    *flush_input = 0;
    pid_t foreground = tcgetpgrp(master);
    /* No foreground is a handled no-op; only a full PTY write needs retry. */
    if (foreground < 0) {
        if (errno == ENOTTY || errno == EIO) return 1;
        return -1;
    }
    if (foreground == 0) return 1;
    struct termios settings;
    if (tcgetattr(slave, &settings) < 0) return -1;
    unsigned char key = 3;
    if (settings.c_lflag & ISIG) {
        errno = 0;
        long disabled = fpathconf(slave, _PC_VDISABLE);
        if (disabled < 0 && errno != 0) return -1;
        if (disabled < 0 || settings.c_cc[VINTR] != (cc_t)disabled) {
            key = settings.c_cc[VINTR];
            *flush_input = !(settings.c_lflag & NOFLSH);
        }
    }
    if (*flush_input && tcflush(slave, TCIFLUSH) < 0) return -1;
    /* The interrupt key takes priority over queued input, including in raw
       mode where it becomes a literal ETX byte. */
    ssize_t count;
    do {
        count = write(master, &key, 1);
    } while (count < 0 && errno == EINTR);
    if (count < 0 && errno == EAGAIN) return 0;
    if (count != 1) return -1;
    return 1;
}

/* One socket frame: opcode, little-endian u32 length, payload. Input frames
   carry an eight-byte generation then up to 4096 bytes. Interrupt carries
   the next generation; resize carries two little-endian u16 values. */
struct frame {
    unsigned char bytes[5 + 8 + 4096];
    size_t have;
    size_t need;
};

struct input_queue {
    unsigned char bytes[16][4096];
    size_t length[16];
    uint64_t generation[16];
    size_t first;
    size_t count;
    size_t sent;
};

struct interrupt_state {
    uint64_t generation[16];
    size_t first;
    size_t count;
    uint64_t applied_generation;
    uint64_t cutoff;
};

static uint64_t frame_generation(const unsigned char *bytes) {
    uint64_t value = 0;
    for (int index = 7; index >= 0; --index)
        value = (value << 8) | bytes[index];
    return value;
}

static void discard_old_input(struct input_queue *queue, uint64_t cutoff) {
    while (queue->count && queue->generation[queue->first] < cutoff) {
        queue->first = (queue->first + 1) % 16;
        --queue->count;
        queue->sent = 0;
    }
}

static int deliver_interrupt(int master, int slave, struct input_queue *queue,
                             struct interrupt_state *state) {
    uint64_t generation = state->generation[state->first];
    int flush_input;
    int sent = send_interrupt(master, slave, &flush_input);
    if (sent < 0) return -1;
    if (!sent) return 0;
    if (flush_input) {
        state->cutoff = generation;
        discard_old_input(queue, state->cutoff);
    }
    state->applied_generation = generation;
    state->first = (state->first + 1) % 16;
    --state->count;
    return 1;
}

static int read_frame(int socket, struct frame *frame, int slave, int master,
                      struct input_queue *queue,
                      struct interrupt_state *interrupt,
                      struct input_queue *input_queue) {
    ssize_t count = read(socket, frame->bytes + frame->have,
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
        if ((operation == 1 && (length < 8 || length > 4104 || !queue)) ||
            (operation == 2 && length != 4) ||
            (operation == 3 && length != 8) ||
            (operation != 1 && operation != 2 && operation != 3) ||
            (queue && operation != 1)) return -2;
        frame->need += length;
        if (length) return 0;
    }
    if (frame->bytes[0] == 1) {
        uint64_t generation = frame_generation(frame->bytes + 5);
        size_t length = frame->need - 13;
        if (length) {
            if (generation < interrupt->cutoff) goto complete;
            if (queue->count == 16) return -2;
            size_t last = (queue->first + queue->count) % 16;
            memcpy(queue->bytes[last], frame->bytes + 13, length);
            queue->length[last] = length;
            queue->generation[last] = generation;
            ++queue->count;
        }
    } else if (frame->bytes[0] == 2) {
        const unsigned char *payload = frame->bytes + 5;
        uint16_t columns = (uint16_t)(payload[0] | (payload[1] << 8));
        uint16_t rows = (uint16_t)(payload[2] | (payload[3] << 8));
        if (!columns || !rows || terminal_size(slave, columns, rows) < 0)
            return -2;
    } else {
        if (interrupt->count == 16) return -2;
        size_t last = (interrupt->first + interrupt->count) % 16;
        interrupt->generation[last] = frame_generation(frame->bytes + 5);
        ++interrupt->count;
        if (deliver_interrupt(master, slave, input_queue, interrupt) < 0)
            return -2;
    }
complete:
    frame->have = 0;
    frame->need = 5;
    return 0;
}

static int bridge(int master, int *slave, int input, int control,
                  int owner_write, int status_read) {
    struct frame input_frame, control_frame;
    struct input_queue queue;
    memset(&input_frame, 0, sizeof(input_frame));
    memset(&control_frame, 0, sizeof(control_frame));
    memset(&queue, 0, sizeof(queue));
    input_frame.need = control_frame.need = 5;
    int child_status = 0;
    int child_done = 0;
    int input_closed = 0;
    struct interrupt_state interrupt;
    memset(&interrupt, 0, sizeof(interrupt));
    for (;;) {
        if (stopping) return 125;
        while (interrupt.count) {
            int sent = deliver_interrupt(master, *slave, &queue, &interrupt);
            if (sent < 0) return 125;
            if (!sent) break;
        }
        struct pollfd events[4] = {
            { master, POLLIN |
                (!input_closed && queue.count && !interrupt.count &&
                 queue.generation[queue.first] <= interrupt.applied_generation
                     ? POLLOUT : 0), 0 },
            { child_done || input_closed ? -1 : input, queue.count < 16 ? POLLIN : 0, 0 },
            { child_done || input_closed || interrupt.count == 16 ? -1 : control, POLLIN, 0 },
            { child_done ? -1 : status_read, POLLIN, 0 },
        };
        int ready = poll(events, 4, child_done ? 100 : interrupt.count ? 50 : 1000);
        if (ready < 0) {
            if (errno == EINTR) continue;
            fprintf(stderr, "Unix PTY poll failed: %s\n", strerror(errno));
            return 125;
        }
        if (!child_done && events[3].revents) {
            ssize_t count = read(status_read, &child_status, sizeof(child_status));
            if (count != (ssize_t)sizeof(child_status)) {
                fprintf(stderr, "Unix PTY watcher status was incomplete\n");
                return 125;
            }
            if (child_status == -1) {
                fprintf(stderr, "Could not confirm Unix PTY session cleanup\n");
                return 125;
            }
            child_done = 1;
            interrupt.count = 0;
            queue.count = 0;
            close(*slave);
            *slave = -1;
        }
        if (events[0].revents & (POLLIN | POLLHUP | POLLERR)) {
            unsigned char output[8192];
            ssize_t count = read(master, output, sizeof(output));
            if (count > 0) {
                if (write_all(STDOUT_FILENO, output, (size_t)count) < 0) {
                    fprintf(stderr, "Unix PTY output write failed: %s\n", strerror(errno));
                    return 125;
                }
            } else if (count == 0 || errno == EIO) {
                if (child_done) break;
            } else if (errno != EINTR && errno != EAGAIN) {
                fprintf(stderr, "Unix PTY master read failed: %s\n", strerror(errno));
                return 125;
            }
        }
        if (!child_done && !input_closed && interrupt.count < 16 && events[2].revents) {
            int result = read_frame(control, &control_frame, *slave, master,
                                    NULL, &interrupt, &queue);
            if (result < 0) {
                fprintf(stderr, "Unix PTY control read or command failed: %s\n",
                        strerror(errno));
                return 125;
            }
        }
        if ((events[0].revents & POLLOUT) && queue.count &&
            !interrupt.count &&
            queue.generation[queue.first] <= interrupt.applied_generation) {
            ssize_t count = write(master, queue.bytes[queue.first] + queue.sent,
                                  queue.length[queue.first] - queue.sent);
            if (count > 0) {
                queue.sent += (size_t)count;
                if (queue.sent == queue.length[queue.first]) {
                    queue.first = (queue.first + 1) % 16;
                    --queue.count;
                    queue.sent = 0;
                }
            } else if (count < 0 && errno == EIO) {
                /* The slave can close before the watcher publishes the child
                   status. Pending input is undeliverable, but cleanup and the
                   child's exit code still come from the watcher. */
                input_closed = 1;
                queue.count = 0;
                queue.sent = 0;
                interrupt.count = 0;
            } else if (count < 0 && errno != EINTR && errno != EAGAIN) {
                fprintf(stderr, "Unix PTY master write failed: %s\n", strerror(errno));
                return 125;
            }
        }
        if (!child_done && !input_closed && queue.count < 16 && events[1].revents) {
            int result = read_frame(input, &input_frame, *slave, master,
                                    &queue, &interrupt, &queue);
            if (result < 0) {
                if (result == -2)
                    fprintf(stderr, "Unix PTY input frame was invalid\n");
                else
                    fprintf(stderr, "Unix PTY input read failed: %s\n",
                            strerror(errno));
                return 125;
            }
        }
        if (child_done && ready == 0) break;
    }
    close(owner_write);
    if (WIFEXITED(child_status)) return WEXITSTATUS(child_status);
    if (WIFSIGNALED(child_status)) return 128 + WTERMSIG(child_status);
    fprintf(stderr, "Unix PTY child status was invalid\n");
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
    int listener = -1, input = -1, control = -1, master = -1, slave = -1;
    int owner[2] = { -1, -1 }, status[2] = { -1, -1 };
    int bound = 0;
    const char *failure = NULL;
    listener = socket(AF_UNIX, SOCK_STREAM, 0);
    if (listener < 0) { failure = "socket"; goto setup_error; }
    if (bind(listener, (struct sockaddr *)&endpoint, sizeof(endpoint)) < 0) {
        failure = "bind"; goto setup_error;
    }
    bound = 1;
    if (chmod(argv[1], 0600) < 0) { failure = "chmod"; goto setup_error; }
    if (listen(listener, 2) < 0) { failure = "listen"; goto setup_error; }
    input = accept(listener, NULL, NULL);
    if (input < 0) { failure = "accept input"; goto setup_error; }
    control = accept(listener, NULL, NULL);
    if (control < 0) { failure = "accept"; goto setup_error; }
    close(listener);
    listener = -1;
    master = posix_openpt(O_RDWR | O_NOCTTY | O_NONBLOCK);
    if (master < 0) { failure = "posix_openpt"; goto setup_error; }
    if (grantpt(master) < 0) { failure = "grantpt"; goto setup_error; }
    if (unlockpt(master) < 0) { failure = "unlockpt"; goto setup_error; }
    char *slave_name = ptsname(master);
    if (!slave_name) { failure = "ptsname"; goto setup_error; }
    slave = open(slave_name, O_RDWR | O_NOCTTY);
    if (slave < 0) { failure = "open slave"; goto setup_error; }
    if (terminal_size(slave, (uint16_t)columns, (uint16_t)rows) < 0) {
        failure = "TIOCSWINSZ slave"; goto setup_error;
    }
    if (pipe(owner) < 0) { failure = "owner pipe"; goto setup_error; }
    if (pipe(status) < 0) { failure = "status pipe"; goto setup_error; }
    pid_t watcher = fork();
    if (watcher < 0) { failure = "watcher fork"; goto setup_error; }
    if (watcher == 0)
        watch_child(master, slave, owner[0], owner[1], status[0], status[1],
                    input, control, argv + 4);
    close(owner[0]);
    owner[0] = -1;
    close(status[1]);
    status[1] = -1;
    if (set_nonblocking(input) < 0 || set_nonblocking(control) < 0) {
        failure = "socket nonblocking"; goto setup_error;
    }
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = stop;
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    sigaction(SIGINT, &action, NULL);
    sigaction(SIGHUP, &action, NULL);
    signal(SIGPIPE, SIG_IGN);
    int result = bridge(master, &slave, input, control, owner[1], status[0]);
    close(owner[1]);
    close(status[0]);
    close(input);
    close(control);
    close(master);
    if (slave >= 0) close(slave);
    unlink(argv[1]);
    return result;

setup_error:;
    int saved_errno = errno;
    fprintf(stderr, "Could not initialize Unix PTY (%s): %s\n",
            failure, strerror(saved_errno));
    if (listener >= 0) close(listener);
    if (input >= 0) close(input);
    if (control >= 0) close(control);
    if (master >= 0) close(master);
    if (slave >= 0) close(slave);
    if (owner[0] >= 0) close(owner[0]);
    if (owner[1] >= 0) close(owner[1]);
    if (status[0] >= 0) close(status[0]);
    if (status[1] >= 0) close(status[1]);
    if (bound) unlink(argv[1]);
    return 125;
}
