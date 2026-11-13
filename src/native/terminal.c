#define _DEFAULT_SOURCE
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

static volatile sig_atomic_t interrupted;
static void stop(int signal_number) { interrupted = signal_number; }
static const char leave[] = "\033[0m\033[?2026l\033[?1006l\033[?1000l\033[?2004l\033[<u\033[?1049l\033[?25h";
static const char save_title[] = "\033[22;2t";
static const char set_title[] = "\033]2;Jecode\007";
static const char restore_title[] = "\033[23;2t";

static int write_all(int fd, const char *bytes, size_t length) {
    while (length) {
        ssize_t written = write(fd, bytes, length);
        if (written < 0 && errno == EINTR) continue;
        if (written <= 0) return -1;
        bytes += written;
        length -= (size_t)written;
    }
    return 0;
}

static int install_signals(void) {
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = stop;
    sigemptyset(&action.sa_mask);
    return sigaction(SIGINT, &action, NULL) || sigaction(SIGTERM, &action, NULL) || sigaction(SIGHUP, &action, NULL);
}

static int read_line(int secret) {
    struct termios saved, hidden;
    if (tcgetattr(STDIN_FILENO, &saved) < 0 || install_signals()) return 2;
    hidden = saved;
    hidden.c_lflag &= ~ECHO;
    if (secret && tcsetattr(STDIN_FILENO, TCSANOW, &hidden) < 0) return 2;
    char *line = NULL;
    size_t capacity = 0;
    ssize_t count = getline(&line, &capacity, stdin);
    int result = count < 0 ? 3 : 0;
    if (secret && tcsetattr(STDIN_FILENO, TCSANOW, &saved) < 0) result = 2;
    if (interrupted) result = 128 + interrupted;
    if (!result && secret && fwrite(line, 1, (size_t)count, stdout) != (size_t)count) result = 2;
    free(line);
    return result;
}

static void note_resize(int signal_number) { (void)signal_number; }

static void measure(int tty, struct winsize *size) {
    if (ioctl(tty, TIOCGWINSZ, size) < 0) memset(size, 0, sizeof(*size));
}

/* Stays until its parent closes stdin, reporting each new size on stdout as
   "ROWS COLUMNS". SIGWINCH ends the wait at once; the timeout only covers a resize
   that lands just before poll starts. */
static void report_sizes(int tty, struct winsize size) {
    struct pollfd control = { STDIN_FILENO, POLLIN, 0 };
    while (!interrupted) {
        int polled = poll(&control, 1, 250);
        if (polled > 0 || (polled < 0 && errno != EINTR)) return;
        struct winsize next;
        measure(tty, &next);
        if (!next.ws_row || !next.ws_col) continue;
        if (next.ws_row == size.ws_row && next.ws_col == size.ws_col) continue;
        size = next;
        printf("%u %u\n", size.ws_row, size.ws_col);
        fflush(stdout);
    }
}

static int guard(void) {
    int tty = open("/dev/tty", O_RDWR | O_CLOEXEC);
    if (tty < 0) return 2;
    struct termios saved, raw;
    if (tcgetattr(tty, &saved) < 0 || install_signals()) { close(tty); return 2; }
    struct sigaction resize;
    memset(&resize, 0, sizeof(resize));
    resize.sa_handler = note_resize;
    sigemptyset(&resize.sa_mask);
    /* A parent that stopped reading must not keep this process from restoring. */
    if (sigaction(SIGWINCH, &resize, NULL) || signal(SIGPIPE, SIG_IGN) == SIG_ERR) { close(tty); return 2; }
    raw = saved;
    cfmakeraw(&raw);
    raw.c_iflag &= ~(IXON | IXOFF);
    raw.c_oflag |= OPOST | ONLCR;
    raw.c_cc[VMIN] = 0;
    raw.c_cc[VTIME] = 1;
    int result = 0;
    if (tcsetattr(tty, TCSANOW, &raw) < 0) { close(tty); return 2; }
    /* `>5u` asks for keys with modifiers to be reported unambiguously, with their
       base layout key so shortcuts work on non-Latin keyboards; terminals without
       the protocol ignore it. Each screen keeps its own request, so it comes after
       switching screens and ends before switching back. The window title is saved
       on the terminal's title stack, set to Jecode and popped on exit; terminals
       ignore what they do not support or what their settings suppress. */
    const char enter[] = "\033[?1049h\033[?2004h\033[?1000h\033[?1006h\033[>5u\033[?25l";
    /* Windows Terminal passes WT_SESSION into WSL but supports setting a title
       without the stack needed to restore it. */
    int title_enabled = getenv("WT_SESSION") == NULL;
    int title_saved = 0;
    if (title_enabled) {
        title_saved = write_all(tty, save_title, sizeof(save_title) - 1) == 0;
        if (!title_saved || write_all(tty, set_title, sizeof(set_title) - 1) < 0) result = 2;
    }
    if (!result && write_all(tty, enter, sizeof(enter) - 1) < 0) result = 2;
    if (!result) {
        struct winsize size;
        measure(tty, &size);
        printf("READY %u %u\n", size.ws_row, size.ws_col);
        fflush(stdout);
        report_sizes(tty, size);
    }
    if (write_all(tty, leave, sizeof(leave) - 1) < 0) result = 2;
    /* Exit code 4 tells the parent that a pushed title still needs a pop. */
    if (title_saved && write_all(tty, restore_title, sizeof(restore_title) - 1) < 0) result = 4;
    if (tcsetattr(tty, TCSANOW, &saved) < 0 && result != 4) result = 2;
    close(tty);
    return result == 4 ? 4 : (interrupted ? 128 + interrupted : result);
}

static int mode(int argc, char **argv) {
    struct termios value;
    memset(&value, 0, sizeof(value));
    if (tcgetattr(STDIN_FILENO, &value) < 0) return 2;
    if (argc == 1 && !strcmp(argv[0], "-g")) {
        /* Keep local flags separate so Darwin's transient PENDIN bit can be
           compared without changing any other part of the saved state. */
        tcflag_t local_flags = value.c_lflag;
        value.c_lflag = 0;
        const unsigned char *bytes = (const unsigned char *)&value;
        printf("v2:lflag=%lx:", (unsigned long)local_flags);
        for (size_t i = 0; i < sizeof(value); ++i) printf("%02x", bytes[i]);
        putchar('\n');
        return 0;
    }
    if (argc == 1 && (!strncmp(argv[0], "v1:", 3) || !strncmp(argv[0], "v2:lflag=", 9))) {
        const char *hex = argv[0] + 3;
        unsigned long local_flags = 0;
        int separate_flags = !strncmp(argv[0], "v2:lflag=", 9);
        if (separate_flags) {
            char *end;
            errno = 0;
            local_flags = strtoul(argv[0] + 9, &end, 16);
            if (errno || end == argv[0] + 9 || *end != ':' ||
                (unsigned long)(tcflag_t)local_flags != local_flags) return 2;
            hex = end + 1;
        }
        if (strlen(hex) != sizeof(value) * 2) return 2;
        unsigned char *bytes = (unsigned char *)&value;
        for (size_t i = 0; i < sizeof(value); ++i) {
            char piece[3] = { hex[i * 2], hex[i * 2 + 1], 0 }, *end;
            unsigned long byte = strtoul(piece, &end, 16);
            if (*end || byte > 255) return 2;
            bytes[i] = (unsigned char)byte;
        }
        if (separate_flags) value.c_lflag = (tcflag_t)local_flags;
    } else {
        for (int i = 0; i < argc; ++i) {
            if (!strcmp(argv[i], "raw")) cfmakeraw(&value);
            else if (!strcmp(argv[i], "-icanon")) value.c_lflag &= ~ICANON;
            else if (!strcmp(argv[i], "-echo")) value.c_lflag &= ~ECHO;
#ifdef PENDIN
            else if (!strcmp(argv[i], "-pendin")) value.c_lflag &= ~PENDIN;
#endif
            else if ((!strcmp(argv[i], "min") || !strcmp(argv[i], "time")) && i + 1 < argc) {
                int key = !strcmp(argv[i], "min") ? VMIN : VTIME;
                char *end;
                long number = strtol(argv[++i], &end, 10);
                if (*end || number < 0 || number > 255) return 2;
                value.c_cc[key] = (cc_t)number;
            } else return 2;
        }
    }
    return tcsetattr(STDIN_FILENO, TCSANOW, &value) < 0 ? 2 : 0;
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "guard")) return guard();
    if (argc == 2 && !strcmp(argv[1], "secret")) return read_line(1);
    if (argc == 2 && !strcmp(argv[1], "wait-enter")) return read_line(0);
    if (argc >= 3 && !strcmp(argv[1], "mode")) return mode(argc - 2, argv + 2);
    return 2;
}
