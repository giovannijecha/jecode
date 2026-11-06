/* Workspace confinement for a guarded command: Landlock on Linux, Seatbelt on
   macOS. Jecode generates the complete policy; this file only applies it. A
   command must never run unconfined after a failure, so errors are returned. */
#define CONFINE_MARKER "JECODE_INTERNAL_SANDBOX_POLICY"
/* Prefixes loader variables on their way through this helper; see restore_deferred. */
#define CONFINE_DEFERRED "JECODE_INTERNAL_DEFERRED_"

#if defined(__linux__)
#include <stdint.h>
#include <sys/prctl.h>

/* Kernel interface values, absent from the older system headers Jecode builds on. */
enum { CREATE_RULESET = 444, ADD_RULE = 445, RESTRICT_SELF = 446 };
#define RULESET_VERSION 1UL
#define RULE_PATH_BENEATH 1
#define RIGHT_EXECUTE (1ULL << 0)
#define RIGHT_WRITE_FILE (1ULL << 1)
#define RIGHT_READ_FILE (1ULL << 2)
#define RIGHT_READ_DIR (1ULL << 3)
#define RIGHT_REFER (1ULL << 13)
#define RIGHT_TRUNCATE (1ULL << 14)
#define FIRST_ABI_RIGHTS ((1ULL << 13) - 1)
#define READ_RIGHTS (RIGHT_EXECUTE | RIGHT_READ_FILE | RIGHT_READ_DIR)
#define FILE_RIGHTS (RIGHT_EXECUTE | RIGHT_WRITE_FILE | RIGHT_READ_FILE | RIGHT_TRUNCATE)
#define SCOPE_ABSTRACT_SOCKET (1ULL << 0)
#define SCOPE_SIGNAL (1ULL << 1)

struct ruleset_attributes {
    uint64_t handled_filesystem;
    uint64_t handled_network;
    uint64_t scoped;
};

struct path_beneath {
    uint64_t allowed;
    int32_t parent;
} __attribute__((packed));

/* A path missing on this system simply grants nothing. */
static int add_rule(int ruleset, const char *path, uint64_t rights) {
    int file = open(path, O_PATH | O_CLOEXEC);
    if (file < 0) return errno == ENOENT || errno == ENOTDIR || errno == EACCES ? 0 : -1;
    struct stat metadata;
    if (fstat(file, &metadata) < 0) { close(file); return -1; }
    /* The kernel rejects directory rights on a file. */
    struct path_beneath rule = { S_ISDIR(metadata.st_mode) ? rights : rights & FILE_RIGHTS, file };
    long result = syscall(ADD_RULE, ruleset, RULE_PATH_BENEATH, &rule, 0);
    close(file);
    return result < 0 ? -1 : 0;
}

/* Reverses the escaping of `%` and line breaks in place. */
static int decode_path(char *path) {
    char *out = path;
    for (char *in = path; *in; in++) {
        if (*in != '%') { *out++ = *in; continue; }
        if (in[1] == '2' && in[2] == '5') *out++ = '%';
        else if (in[1] == '0' && in[2] == 'A') *out++ = '\n';
        else return -1;
        in += 2;
    }
    *out = 0;
    return 0;
}

/* Each policy line is `r<path>` for read and execute or `w<path>` for full access. */
static int confine(char *policy) {
    long abi = syscall(CREATE_RULESET, NULL, 0, RULESET_VERSION);
    if (abi < 1) {
        fprintf(stderr, "Landlock is unavailable: %s", strerror(errno));
        return -1;
    }
    uint64_t write_rights = FIRST_ABI_RIGHTS | (abi >= 2 ? RIGHT_REFER : 0) | (abi >= 3 ? RIGHT_TRUNCATE : 0);
    /* Older kernels accept only the fields they know. */
    struct ruleset_attributes attributes = { write_rights, 0, abi >= 6 ? SCOPE_SIGNAL | SCOPE_ABSTRACT_SOCKET : 0 };
    size_t size = abi >= 6 ? sizeof attributes : sizeof attributes.handled_filesystem;
    int ruleset = (int)syscall(CREATE_RULESET, &attributes, size, 0UL);
    if (ruleset < 0) {
        fprintf(stderr, "Could not create a Landlock ruleset: %s", strerror(errno));
        return -1;
    }
    int result = 0;
    for (char *line = policy; line && *line && !result;) {
        char *end = strchr(line, '\n');
        if (end) *end = 0;
        char kind = *line;
        if ((kind != 'r' && kind != 'w') || decode_path(line + 1) < 0) {
            fprintf(stderr, "Invalid Landlock policy line");
            result = -1;
        } else if (add_rule(ruleset, line + 1, kind == 'r' ? READ_RIGHTS : write_rights) < 0) {
            fprintf(stderr, "Could not add a Landlock rule for %s: %s", line + 1, strerror(errno));
            result = -1;
        }
        line = end ? end + 1 : NULL;
    }
    /* Required for an unprivileged process; also stops set-user-ID programs regaining access. */
    if (!result && prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0) {
        fprintf(stderr, "Could not forbid new privileges: %s", strerror(errno));
        result = -1;
    }
    if (!result && syscall(RESTRICT_SELF, ruleset, 0UL) < 0) {
        fprintf(stderr, "Could not apply the Landlock ruleset: %s", strerror(errno));
        result = -1;
    }
    close(ruleset);
    return result;
}

#elif defined(__APPLE__)
#include <stdint.h>

/* Public libSystem entry points; `sandbox.h` marks them deprecated with no replacement. */
extern int sandbox_init(const char *profile, uint64_t flags, char **error);
extern void sandbox_free_error(char *error);

static int confine(char *policy) {
    char *error = NULL;
    if (sandbox_init(policy, 0, &error) == 0) return 0;
    fprintf(stderr, "Seatbelt rejected the policy: %s", error ? error : "unknown error");
    if (error) sandbox_free_error(error);
    return -1;
}
#endif
