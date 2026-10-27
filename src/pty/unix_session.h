static void kill_session(pid_t child) {
    /* The direct PID also covers the instant before the child calls setsid. */
    if (child > 0) {
        (void)kill(-child, SIGKILL);
        (void)kill(child, SIGKILL);
    }
}

/* POSIX has no signal-to-session operation. /proc supplies Linux candidates,
   while macOS uses its system ps. getsid verifies each before signaling.
   The session leader remains unreaped, so its session ID cannot be reused. */
#ifdef __linux__
static int sweep_session(pid_t leader, int *active) {
    DIR *entries = opendir("/proc");
    if (!entries) return -1;
    *active = 0;
    int failure = 0;
    struct dirent *entry;
    while ((entry = readdir(entries))) {
        char *end;
        long number = strtol(entry->d_name, &end, 10);
        if (*end || number <= 0 || number > INT_MAX || number == leader)
            continue;
        pid_t pid = (pid_t)number;
        if (getsid(pid) != leader) continue;
        char path[64], line[4096];
        if (snprintf(path, sizeof(path), "/proc/%ld/stat", number) >=
            (int)sizeof(path)) continue;
        FILE *stat = fopen(path, "r");
        if (!stat) {
            if (errno != ENOENT && errno != ESRCH) failure = 1;
            continue;
        }
        int zombie = fgets(line, sizeof(line), stat) &&
            strrchr(line, ')') && strrchr(line, ')')[2] == 'Z';
        if (ferror(stat)) failure = 1;
        fclose(stat);
        if (zombie) continue;
        if (getsid(pid) != leader) continue;
        ++*active;
        if (kill(pid, SIGKILL) < 0 && errno != ESRCH) failure = 1;
    }
    if (closedir(entries) < 0) failure = 1;
    return failure ? -1 : 0;
}
#else
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
#endif

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

static void target(int master, int slave, int owner_read,
                   int owner_write, int status_write, int input, int control,
                   char **argv) {
    close(master);
    close(owner_read);
    close(owner_write);
    close(status_write);
    close(input);
    close(control);
    if (setsid() < 0) _exit(126);
    signal(SIGINT, SIG_DFL);
    signal(SIGQUIT, SIG_DFL);
    signal(SIGTERM, SIG_DFL);
    signal(SIGHUP, SIG_DFL);
    signal(SIGPIPE, SIG_DFL);
    if (ioctl(slave, TIOCSCTTY, 0) < 0) _exit(126);
    if (dup2(slave, STDIN_FILENO) < 0 || dup2(slave, STDOUT_FILENO) < 0 ||
        dup2(slave, STDERR_FILENO) < 0) _exit(126);
    if (slave > STDERR_FILENO) close(slave);
    execvp(argv[0], argv);
    _exit(errno == ENOENT ? 127 : 126);
}

static void watch_child(int master, int slave, int owner_read,
                        int owner_write, int status_read, int status_write,
                        int input, int control, char **argv) {
    close(master);
    close(input);
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
        target(master, slave, owner_read, owner_write, status_write,
               input, control, argv);
    close(slave);

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
