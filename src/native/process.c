#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <sys/types.h>
#include <unistd.h>

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
    if (source < 0 || dup2(source, STDIN_FILENO) < 0) { free(program); return 125; }
    if (source > STDERR_FILENO) close(source);
    if (write(STDOUT_FILENO, "READY\n", 6) != 6) { free(program); return 125; }
    execv(program, argv);
    free(program);
    return errno == ENOENT ? 127 : 126;
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
    if (argc == 3 && !strcmp(argv[1], "space")) {
        struct statvfs volume;
        if (statvfs(argv[2], &volume) < 0) return 1;
        unsigned long long free_bytes = (unsigned long long)volume.f_bavail * volume.f_frsize;
        printf("%llu\n", free_bytes);
        return 0;
    }
    return 2;
}
