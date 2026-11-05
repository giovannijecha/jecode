// Linux HTTP/1.1 adapter over OS sockets and the OS-supplied OpenSSL TLS ABI. It
// serves requests in order on reused connections and exits when stdin closes or
// after an idle period. Its protocol is winhttp.cs's. No external program or
// development headers.
#define _GNU_SOURCE /* strcasestr */
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>

#include "linux_http_net.h"

// Servers close idle connections on their own schedule, so a longer idle helper
// would mostly keep a process alive for connections already gone.
enum { IDLE_LIMIT_MS = 300000, CONNECT_LIMIT_MS = 10000, RECEIVE_LIMIT_MS = 120000, STALE_RETRIES = 2 };

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

static int decimal(const char *text, unsigned long long *value) {
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
    int valid = decimal(text, &count) && count <= 1000;
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

#include "linux_http_socket.h"

struct exchange {
    char *url, *method;
    struct header *headers;
    long header_count;
    unsigned long long timeout_ms;
    unsigned char *body;
    size_t body_size;
};

static void free_exchange(struct exchange *exchange) {
    free(exchange->url);
    free(exchange->method);
    if (exchange->headers) free_headers(exchange->headers, exchange->header_count);
    free(exchange->body);
    memset(exchange, 0, sizeof(*exchange));
}

// 1 with a whole request, 0 at the end of stdin; -1 when it is invalid.
static int read_exchange(struct exchange *exchange) {
    char *first = read_line(IDLE_LIMIT_MS);
    if (!first) return 0;
    exchange->url = decoded_text(first);
    if (!exchange->url) return -1;
    exchange->method = decoded_line();
    char *timeout = read_line(-1);
    int timed = decimal(timeout, &exchange->timeout_ms);
    free(timeout);
    exchange->header_count = read_header_count();
    if (!http_token(exchange->method) || !timed || exchange->header_count < 0) return -1;
    exchange->headers = read_headers(exchange->header_count);
    if (!exchange->headers) return -1;
    char *length = read_line(-1);
    unsigned long long body_size = 0;
    int sized = decimal(length, &body_size) && body_size < SIZE_MAX;
    free(length);
    if (!sized) return -1;
    exchange->body_size = (size_t)body_size;
    exchange->body = malloc(exchange->body_size ? exchange->body_size : 1);
    if (!exchange->body || !read_input(exchange->body, exchange->body_size)) return -1;
    return 1;
}

static int send_request(struct channel *channel, const struct exchange *exchange, const struct target *target) {
    size_t capacity = 4096, used = 0;
    for (long i = 0; i < exchange->header_count; i++)
        capacity += strlen(exchange->headers[i].name) + strlen(exchange->headers[i].value) + 4;
    capacity += strlen(exchange->url);
    char *head = malloc(capacity);
    if (!head) return fail('E', "Native HTTP is out of memory");
    const char *leading_slash = *target->path == '?' ? "/" : "";
    used += (size_t)snprintf(head + used, capacity - used, "%s %s%s HTTP/1.1\r\nHost: %.*s\r\n", exchange->method,
        target->proxy && !target->secure ? exchange->url : leading_slash,
        target->proxy && !target->secure ? "" : target->path,
        (int)target->authority_length, target->authority);
    int has_length = 0;
    for (long i = 0; i < exchange->header_count; i++) {
        if (!strcasecmp(exchange->headers[i].name, "Content-Length")) has_length = 1;
        used += (size_t)snprintf(head + used, capacity - used, "%s: %s\r\n",
            exchange->headers[i].name, exchange->headers[i].value);
    }
    if (!has_length && (exchange->body_size || !strcasecmp(exchange->method, "POST") ||
        !strcasecmp(exchange->method, "PUT") || !strcasecmp(exchange->method, "PATCH")))
        used += (size_t)snprintf(head + used, capacity - used, "Content-Length: %zu\r\n", exchange->body_size);
    used += (size_t)snprintf(head + used, capacity - used, "\r\n");
    int sent = used < capacity && send_bytes(channel, head, used);
    free(head);
    if (!used || used >= capacity) return fail('E', "Native HTTP request is invalid");
    return sent && (!exchange->body_size || send_bytes(channel, exchange->body, exchange->body_size));
}

static int copy_body(struct channel *channel, uint64_t left) {
    unsigned char block[16384];
    while (left) {
        size_t want = left > sizeof(block) ? sizeof(block) : (size_t)left;
        int count = receive_bytes(channel, block, want);
        if (count <= 0) return fail('T', "Native HTTP response is incomplete");
        frame('D', block, (size_t)count);
        left -= (uint64_t)count;
    }
    return 1;
}

static int copy_chunks(struct channel *channel, char *line, size_t capacity) {
    for (;;) {
        if (!receive_line(channel, line, capacity)) return fail('T', "Native HTTP response is incomplete");
        char *end = NULL;
        if (!((*line >= '0' && *line <= '9') || (*line >= 'a' && *line <= 'f') || (*line >= 'A' && *line <= 'F')))
            return fail('E', "Native HTTP chunk is invalid");
        errno = 0;
        unsigned long long chunk = strtoull(line, &end, 16);
        if (errno == ERANGE) return fail('E', "Native HTTP chunk is invalid");
        if (*end == ';') {
            size_t extension = strlen(end);
            if (extension < 3 || strcmp(end + extension - 2, "\r\n") != 0) return fail('E', "Native HTTP chunk is invalid");
            for (size_t i = 1; i + 2 < extension; i++) {
                if ((unsigned char)end[i] < 0x20 || (unsigned char)end[i] > 0x7e) return fail('E', "Native HTTP chunk is invalid");
            }
        } else if (strcmp(end, "\r\n") != 0) return fail('E', "Native HTTP chunk is invalid");
        if (chunk == 0) {
            do {
                if (!receive_line(channel, line, capacity)) return fail('T', "Native HTTP response is incomplete");
            } while (strcmp(line, "\r\n") != 0);
            return 1;
        }
        if (!copy_body(channel, chunk)) return 0;
        char ending[2];
        if (!receive_exact(channel, ending, 2) || memcmp(ending, "\r\n", 2)) return fail('T', "Native HTTP response is incomplete");
    }
}

struct head { char *text; size_t size; int status, chunked, has_length, closes; uint64_t length; };

// The final response head, past interim ones. `empty` tells a closed connection
// from a broken response, which matters on a reused connection.
static int read_head(struct channel *channel, struct head *head, char *line, size_t capacity, int *empty) {
    *empty = 1;
    for (;;) {
        size_t used = 0;
        head->chunked = head->has_length = 0;
        head->length = 0;
        if (!receive_line(channel, line, capacity)) return fail('T', "Native HTTP response ended before its headers");
        *empty = 0;
        if (sscanf(line, "HTTP/%*s %d", &head->status) != 1 || head->status < 100 || head->status > 599)
            return fail('T', "Native HTTP response status is invalid");
        head->closes = strncmp(line, "HTTP/1.1", 8) != 0;
        do {
            size_t n = strlen(line);
            if (used + n > 65536) return fail('E', "Native HTTP response headers are too large");
            memcpy(head->text + used, line, n);
            used += n;
            if (strcmp(line, "\r\n") == 0) break;
            if (!strncasecmp(line, "Transfer-Encoding:", 18)) {
                char *value = line + 18;
                while (*value == ' ' || *value == '\t') value++;
                if (strncasecmp(value, "chunked", 7)) return fail('E', "Native HTTP response framing is invalid");
                value += 7;
                while (*value == ' ' || *value == '\t') value++;
                if (strcmp(value, "\r\n") != 0 || head->chunked) return fail('E', "Native HTTP response framing is invalid");
                head->chunked = 1;
            }
            if (!strncasecmp(line, "Content-Length:", 15)) {
                char *value = line + 15, *end = NULL;
                while (*value == ' ' || *value == '\t') value++;
                if (*value < '0' || *value > '9') return fail('E', "Native HTTP response length is invalid");
                errno = 0;
                unsigned long long parsed = strtoull(value, &end, 10);
                while (*end == ' ' || *end == '\t') end++;
                if (errno == ERANGE || strcmp(end, "\r\n") != 0 || (head->has_length && head->length != parsed))
                    return fail('E', "Native HTTP response length is invalid");
                head->length = parsed;
                head->has_length = 1;
            }
            if (!strncasecmp(line, "Connection:", 11) && strcasestr(line + 11, "close")) head->closes = 1;
        } while (receive_line(channel, line, capacity));
        if (used < 4 || memcmp(head->text + used - 4, "\r\n\r\n", 4) != 0 || (head->chunked && head->has_length))
            return fail('T', "Native HTTP response headers are incomplete");
        if (head->status < 200) {
            if (head->status == 101) return fail('E', "Native HTTP response switched protocols unexpectedly");
            continue;
        }
        head->size = used;
        return 1;
    }
}

// Copies one response. `reusable` says the connection ended exactly at its end.
static int copy_response(struct channel *channel, const struct exchange *exchange, const struct head *head, char *line, size_t capacity, int *reusable) {
    frame('D', head->text, head->size);
    if (!strcasecmp(exchange->method, "HEAD") || head->status == 204 || head->status == 304) {
        *reusable = !head->closes;
        return 1;
    }
    if (head->chunked) {
        if (!copy_chunks(channel, line, capacity)) return 0;
        *reusable = !head->closes;
        return 1;
    }
    if (head->has_length) {
        if (!copy_body(channel, head->length)) return 0;
        *reusable = !head->closes;
        return 1;
    }
    unsigned char block[16384];
    for (;;) {
        int count = receive_bytes(channel, block, sizeof(block));
        if (count == 0) return 1;
        if (count < 0) return fail('T', "Native HTTP response is incomplete");
        frame('D', block, (size_t)count);
    }
}

static int safe_to_repeat(const char *method) {
    return !strcasecmp(method, "GET") || !strcasecmp(method, "HEAD");
}

static int perform(const struct exchange *exchange) {
    struct target target;
    int result = 0;
    char *line = malloc(65536);
    struct head head = { .text = malloc(65536) };
    if (!line || !head.text || !parse_target(exchange->url, &target)) goto done;
    unsigned long long timeout = exchange->timeout_ms;
    int connect_ms = timeout && timeout < CONNECT_LIMIT_MS ? (int)timeout : CONNECT_LIMIT_MS;
    // Without a total limit, the per-operation limit still bounds silence.
    operation_limit_ms = timeout && timeout < RECEIVE_LIMIT_MS ? (int)timeout : RECEIVE_LIMIT_MS;
    for (int attempt = 0;; attempt++) {
        struct channel *channel = take_cached(target.key);
        int reused = channel != NULL;
        if (!channel) channel = open_channel(&target, connect_ms);
        if (!channel) break;
        set_socket_limits(channel, operation_limit_ms);
        int empty = 1, reusable = 0;
        int sent = send_request(channel, exchange, &target);
        if (sent && read_head(channel, &head, line, 65536, &empty)) {
            result = copy_response(channel, exchange, &head, line, 65536, &reusable);
            if (result && reusable) keep_channel(channel);
            else close_channel(channel);
            break;
        }
        close_channel(channel);
        // A failed receive does not prove that the server did not process a
        // request, so only safe methods may be repeated on a new connection.
        if (!(reused && empty && attempt < STALE_RETRIES && safe_to_repeat(exchange->method))) break;
    }
    free_target(&target);
done:
    free(line);
    free(head.text);
    return result;
}

int main(int argc, char **argv) {
    // Writes to a closed connection report an error instead of ending the helper.
    signal(SIGPIPE, SIG_IGN);
    if (argc == 3 && !strcmp(argv[1], "socket")) return serve_socket(argv[2]);
    for (;;) {
        struct exchange exchange = {0};
        failure_tag = 'E';
        failure = "Native HTTP request is invalid";
        int state = read_exchange(&exchange);
        if (state == 0) { free_exchange(&exchange); return 0; }
        frame('A', NULL, 0);
        if (state < 0) {
            frame('E', failure, strlen(failure));
            return 1;
        }
        failure = "Native HTTP request failed";
        if (perform(&exchange)) frame('E', NULL, 0);
        else frame(failure_tag, failure, strlen(failure));
        free_exchange(&exchange);
    }
}
