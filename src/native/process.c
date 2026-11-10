#ifdef __linux__
#define _GNU_SOURCE /* O_PATH and syscall(), for Landlock. */
#else
#define _POSIX_C_SOURCE 200809L
#endif
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "process_confine.h"

extern char **environ;

/* With a controlling terminal a confined command could type into Jecode or the
   user's shell through TIOCSTI. This process never leads a session, so only it and
   its future children let go of the terminal. */
static int leave_terminal(void) {
    int terminal = open("/dev/tty", O_RDWR | O_NOCTTY | O_CLOEXEC);
    if (terminal < 0) return 0;
    int result = ioctl(terminal, TIOCNOTTY);
    close(terminal);
    if (result < 0) fprintf(stderr, "Could not leave the controlling terminal: %s", strerror(errno));
    return result;
}

/* The dynamic loader acts on its variables before main, outside the policy. They
   reach this helper under another name and take effect only for the command. */
static int restore_deferred(void) {
    size_t length = strlen(CONFINE_DEFERRED);
    for (char **entry = environ; *entry;) {
        if (strncmp(*entry, CONFINE_DEFERRED, length)) { entry++; continue; }
        char *carried = strdup(*entry), *value = carried ? strchr(carried, '=') : NULL;
        if (!value) { free(carried); return -1; }
        *value++ = 0;
        int failed = setenv(carried + length, value, 1) < 0 || unsetenv(carried) < 0;
        free(carried);
        if (failed) return -1;
        entry = environ; /* Both calls can move the array. */
    }
    return 0;
}

/* This adapter uses libc directly; no command shell or utilities are needed. */
static char *resolve(const char *program) {
    if (strchr(program, '/')) return strdup(program);
    const char *path = getenv("PATH");
    if (!path) path = "/usr/bin:/bin";
    char *paths = strdup(path);
    if (!paths) return NULL;
    char *entry = paths;
    for (;;) {
        char *end = strchr(entry, ':');
        if (end) *end = 0;
        size_t size = strlen(entry) + strlen(program) + 3;
        char *candidate = malloc(size);
        if (!candidate) { free(paths); return NULL; }
        snprintf(candidate, size, "%s/%s", *entry ? entry : ".", program);
        struct stat metadata;
        if (access(candidate, X_OK) == 0 && stat(candidate, &metadata) == 0 && S_ISREG(metadata.st_mode)) {
            free(paths);
            return candidate;
        }
        free(candidate);
        if (!end) break;
        entry = end + 1;
    }
    free(paths);
    return NULL;
}

/* The named FIFO belongs to the owner, but reopening it as /dev/stdin can
   block after its writer closes. Forward it into an anonymous pipe instead. */
static void relay_input(int source, int output) {
    close(STDIN_FILENO);  /* Do not keep the owner's control channel open. */
    close(STDOUT_FILENO); /* Do not keep the target's capture pipes open. */
    close(STDERR_FILENO);
    char bytes[8192];
    for (;;) {
        ssize_t count = read(source, bytes, sizeof bytes);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) break;
        for (ssize_t sent = 0; sent < count;) {
            ssize_t part = write(output, bytes + sent, (size_t)(count - sent));
            if (part < 0 && errno == EINTR) continue;
            if (part <= 0) _exit(0);
            sent += part;
        }
    }
    close(source);
    close(output);
    _exit(0);
}

static int guard(const char *input, char **argv) {
    char *program = resolve(argv[0]);
    struct stat metadata;
    if (!program || stat(program, &metadata) < 0 || !S_ISREG(metadata.st_mode) || access(program, X_OK) < 0) {
        free(program);
        return 127;
    }
    pid_t group = getpgrp();
    pid_t guardian = fork();
    if (guardian < 0) { free(program); return 125; }
    if (guardian == 0) {
        int null = open("/dev/null", O_WRONLY);
        if (null < 0 || dup2(null, STDOUT_FILENO) < 0 || dup2(null, STDERR_FILENO) < 0) _exit(125);
        if (null > STDERR_FILENO) close(null);
        char byte;
        while (read(STDIN_FILENO, &byte, 1) < 0 && errno == EINTR) {}
        if (strcmp(input, "/dev/null")) {
            (void)unlink(input);
            char *folder = strdup(input);
            if (folder) {
                char *slash = strrchr(folder, '/');
                if (slash) { *slash = 0; (void)rmdir(folder); }
                free(folder);
            }
        }
        (void)kill(-group, SIGKILL);
        _exit(0);
    }
    int source = open(input, O_RDONLY);
    if (source < 0) { free(program); return 125; }
    if (strcmp(input, "/dev/null")) {
        int data[2];
        if (pipe(data) < 0) { close(source); free(program); return 125; }
        pid_t relay = fork();
        if (relay < 0) { close(data[0]); close(data[1]); close(source); free(program); return 125; }
        if (relay == 0) { close(data[0]); relay_input(source, data[1]); }
        close(source);
        close(data[1]);
        source = data[0];
    }
    if (dup2(source, STDIN_FILENO) < 0) { close(source); free(program); return 125; }
    if (source > STDERR_FILENO) close(source);
    /* The guardian above stays outside: it must still clean up and signal the group. */
    char *policy = getenv(CONFINE_MARKER);
    int confined = policy != NULL;
    if (confined) {
        policy = strdup(policy);
        unsetenv(CONFINE_MARKER);
        if (!policy || leave_terminal() < 0 || confine(policy) < 0 || restore_deferred() < 0) {
            /* Reported to the owner in place of READY; the target never starts. */
            if (write(STDOUT_FILENO, "NOBOX\n", 6) != 6) {}
            free(program);
            return 125;
        }
        free(policy);
    }
    if (write(STDOUT_FILENO, "READY\n", 6) != 6) { free(program); return 125; }
    execv(program, argv);
    int failure = errno;
    if (confined && (failure == EACCES || failure == EPERM)) {
        fprintf(stderr, "%s: outside what Workspace confinement allows to run\n", program);
    }
    free(program);
    return failure == ENOENT ? 127 : 126;
}

int main(int argc, char **argv) {
    if (argc >= 4 && !strcmp(argv[1], "guard")) return guard(argv[2], &argv[3]);
    if (argc == 3 && !strcmp(argv[1], "fifo")) return mkfifo(argv[2], 0600) < 0 ? 1 : 0;
    if (argc == 3 && !strcmp(argv[1], "kill-group")) {
        char *end;
        long group = strtol(argv[2], &end, 10);
        if (*end || group <= 1 || group > INT_MAX) return 2;
        return kill(-(pid_t)group, SIGKILL) == 0 || errno == ESRCH ? 0 : 1;
    }
    return 2;
}
