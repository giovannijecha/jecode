// Framed stdin/stdout shared by the owned Unix HTTP adapters.
#include <poll.h>
// What the current request's failure reports: 'T' when it may clear on retry.
static const char *failure = "Native HTTP request failed";
static char failure_tag = 'E';
static int fail(char tag, const char *message) {
    failure_tag = tag;
    failure = message;
    return 0;
}

// Bounds silence on the current request, as WinHTTP's per-operation limits do.
static int operation_limit_ms = 120000;

static unsigned char input[65536];
static size_t input_start, input_end;

// 1 with data, 0 at the end of stdin, -1 after the limit passed with nothing to read.
static int fill_input(int limit_ms) {
    struct pollfd ready = { STDIN_FILENO, POLLIN, 0 };
    int polled;
    do polled = poll(&ready, 1, limit_ms); while (polled < 0 && errno == EINTR);
    if (polled == 0) return -1;
    if (polled < 0) return 0;
    ssize_t count;
    do count = read(STDIN_FILENO, input, sizeof(input)); while (count < 0 && errno == EINTR);
    if (count <= 0) return 0;
    input_start = 0;
    input_end = (size_t)count;
    return 1;
}

// One input line without its ending; NULL at the end of stdin.
static char *read_line(int limit_ms) {
    size_t capacity = 256, used = 0;
    char *text = malloc(capacity);
    while (text) {
        if (input_start == input_end) {
            int filled = fill_input(limit_ms);
            if (filled < 0) _exit(0);
            if (filled == 0) { free(text); return NULL; }
        }
        // Only the first bytes wait for the idle limit; the rest is already on its way.
        limit_ms = -1;
        char byte = (char)input[input_start++];
        if (byte == '\n') {
            if (used && text[used - 1] == '\r') used--;
            text[used] = 0;
            return text;
        }
        if (used + 2 > capacity) {
            char *grown = realloc(text, capacity *= 2);
            if (!grown) { free(text); return NULL; }
            text = grown;
        }
        text[used++] = byte;
    }
    return NULL;
}

static int read_input(void *bytes, size_t size) {
    unsigned char *cursor = bytes;
    while (size) {
        if (input_start == input_end && fill_input(-1) != 1) return 0;
        size_t count = input_end - input_start;
        if (count > size) count = size;
        memcpy(cursor, input + input_start, count);
        input_start += count;
        cursor += count;
        size -= count;
    }
    return 1;
}

static void write_output(const void *bytes, size_t size) {
    const unsigned char *cursor = bytes;
    while (size) {
        ssize_t count = write(STDOUT_FILENO, cursor, size);
        if (count < 0 && errno == EINTR) continue;
        // The parent stopped reading; nothing is left to report to.
        if (count <= 0) _exit(1);
        cursor += count;
        size -= (size_t)count;
    }
}

static void frame(char tag, const void *bytes, size_t count) {
    unsigned char head[5] = { (unsigned char)tag, (unsigned char)count, (unsigned char)(count >> 8),
                              (unsigned char)(count >> 16), (unsigned char)(count >> 24) };
    write_output(head, sizeof(head));
    if (count) write_output(bytes, count);
}

static unsigned char *decode(const char *line, size_t *length) {
    size_t count = strlen(line);
    unsigned char *result = malloc(count * 3 / 4 + 4);
    if (!result) return NULL;
    *length = 0;
    unsigned int bits = 0;
    int available = 0;
    for (size_t i = 0; i < count && line[i] != '='; i++) {
        unsigned char c = (unsigned char)line[i];
        int value = c >= 'A' && c <= 'Z' ? c - 'A'
                  : c >= 'a' && c <= 'z' ? c - 'a' + 26
                  : c >= '0' && c <= '9' ? c - '0' + 52
                  : c == '+' ? 62 : c == '/' ? 63 : -1;
        if (value < 0) { free(result); return NULL; }
        bits = (bits << 6) | (unsigned int)value;
        available += 6;
        if (available >= 8) {
            available -= 8;
            result[(*length)++] = (unsigned char)(bits >> available);
        }
    }
    result[*length] = 0;
    return result;
}

// Takes ownership of a base64 line. Headers and URLs go on the wire one per line,
// so a decoded line break would inject another.
static char *decoded_text(char *raw) {
    if (!raw) return NULL;
    size_t size = 0;
    unsigned char *value = decode(raw, &size);
    free(raw);
    if (value && (memchr(value, 0, size) || memchr(value, '\r', size) || memchr(value, '\n', size))) {
        free(value);
        return NULL;
    }
    return (char *)value;
}

static char *decoded_line(void) {
    return decoded_text(read_line(-1));
}

static int wire_decimal(const char *text, unsigned long long *value) {
    if (!text || *text < '0' || *text > '9') return 0;
    char *end = NULL;
    errno = 0;
    *value = strtoull(text, &end, 10);
    return errno == 0 && *end == 0;
}

struct header { char *name, *value; };

static void free_headers(struct header *headers, long count) {
    for (long i = 0; i < count; i++) { free(headers[i].name); free(headers[i].value); }
    free(headers);
}

static long read_header_count(void) {
    char *text = read_line(-1);
    unsigned long long count = 0;
    int valid = wire_decimal(text, &count) && count <= 1000;
    free(text);
    return valid ? (long)count : -1;
}

static int http_token(const char *text) {
    if (!text || !*text) return 0;
    for (; *text; text++) {
        char c = *text;
        if (!((c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') ||
              strchr("!#$%&'*+-.^_`|~", c))) return 0;
    }
    return 1;
}

static struct header *read_headers(long count) {
    struct header *headers = calloc((size_t)count + 1, sizeof(*headers));
    if (!headers) fail('E', "Native HTTP is out of memory");
    for (long i = 0; headers && i < count; i++) {
        headers[i].name = decoded_line();
        headers[i].value = decoded_line();
        if (!http_token(headers[i].name) || !headers[i].value) {
            fail('E', "Native HTTP request header is invalid");
            free_headers(headers, i + 1);
            return NULL;
        }
    }
    return headers;
}


static void base64_encode(const unsigned char *bytes, size_t size, char *text) {
    static const char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t written = 0;
    for (size_t i = 0; i < size; i += 3) {
        unsigned int block = (unsigned int)bytes[i] << 16;
        if (i + 1 < size) block |= (unsigned int)bytes[i + 1] << 8;
        if (i + 2 < size) block |= bytes[i + 2];
        text[written++] = alphabet[block >> 18 & 63];
        text[written++] = alphabet[block >> 12 & 63];
        text[written++] = i + 1 < size ? alphabet[block >> 6 & 63] : '=';
        text[written++] = i + 2 < size ? alphabet[block & 63] : '=';
    }
    text[written] = 0;
}
