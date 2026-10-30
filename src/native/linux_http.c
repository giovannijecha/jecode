// Linux HTTP/1.1 adapter over OS sockets and the OS-supplied OpenSSL TLS ABI.
// Stdin protocol matches winhttp.cs. No external program or development headers.
#define _POSIX_C_SOURCE 200809L
#include <arpa/inet.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

typedef struct ssl_ctx_st SSL_CTX;
typedef struct ssl_st SSL;
typedef struct ssl_method_st SSL_METHOD;
typedef struct X509_VERIFY_PARAM_st X509_VERIFY_PARAM;
static const SSL_METHOD *(*tls_client_method)(void);
static SSL_CTX *(*ssl_ctx_new)(const SSL_METHOD *);
static void (*ssl_ctx_free)(SSL_CTX *);
static void (*ssl_ctx_verify)(SSL_CTX *, int, void *);
static int (*ssl_ctx_paths)(SSL_CTX *);
static SSL *(*ssl_new)(SSL_CTX *);
static void (*ssl_free)(SSL *);
static int (*ssl_set_fd)(SSL *, int);
static long (*ssl_ctrl)(SSL *, int, long, void *);
static int (*ssl_set1_host)(SSL *, const char *);
static X509_VERIFY_PARAM *(*ssl_get0_param)(SSL *);
static int (*verify_ip)(X509_VERIFY_PARAM *, const char *);
static int (*ssl_connect)(SSL *);
static long (*ssl_verify_result)(const SSL *);
static int (*ssl_write)(SSL *, const void *, int);
static int (*ssl_read)(SSL *, void *, int);

static int load_tls(void) {
    void *library = dlopen("libssl.so.3", RTLD_NOW | RTLD_LOCAL);
    if (!library) library = dlopen("libssl.so.1.1", RTLD_NOW | RTLD_LOCAL);
    if (!library) return 0;
#define SYMBOL(slot, name) do { *(void **)(&slot) = dlsym(library, name); if (!slot) return 0; } while (0)
    SYMBOL(tls_client_method, "TLS_client_method");
    SYMBOL(ssl_ctx_new, "SSL_CTX_new");
    SYMBOL(ssl_ctx_free, "SSL_CTX_free");
    SYMBOL(ssl_ctx_verify, "SSL_CTX_set_verify");
    SYMBOL(ssl_ctx_paths, "SSL_CTX_set_default_verify_paths");
    SYMBOL(ssl_new, "SSL_new");
    SYMBOL(ssl_free, "SSL_free");
    SYMBOL(ssl_set_fd, "SSL_set_fd");
    SYMBOL(ssl_ctrl, "SSL_ctrl");
    SYMBOL(ssl_set1_host, "SSL_set1_host");
    SYMBOL(ssl_get0_param, "SSL_get0_param");
    SYMBOL(verify_ip, "X509_VERIFY_PARAM_set1_ip_asc");
    SYMBOL(ssl_connect, "SSL_connect");
    SYMBOL(ssl_verify_result, "SSL_get_verify_result");
    SYMBOL(ssl_write, "SSL_write");
    SYMBOL(ssl_read, "SSL_read");
#undef SYMBOL
    return 1;
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

static char *line(void) {
    char *result = NULL;
    size_t capacity = 0;
    if (getline(&result, &capacity, stdin) < 0) { free(result); return NULL; }
    size_t n = strlen(result);
    if (n && result[n - 1] == '\n') result[--n] = 0;
    if (n && result[n - 1] == '\r') result[n - 1] = 0;
    return result;
}

static unsigned char *decoded_line(size_t *size) {
    char *raw = line();
    if (!raw) return NULL;
    unsigned char *value = decode(raw, size);
    free(raw);
    return value;
}

static int valid_text(const unsigned char *text, size_t size) {
    return memchr(text, 0, size) == NULL && memchr(text, '\r', size) == NULL &&
           memchr(text, '\n', size) == NULL;
}

static int decimal(const char *text, unsigned long long *value) {
    if (!text || *text < '0' || *text > '9') return 0;
    char *end = NULL;
    errno = 0;
    *value = strtoull(text, &end, 10);
    return errno == 0 && *end == 0;
}

// Bounds each address attempt so a blackholed route cannot stall for minutes.
static int connect_within(int socket_fd, const struct sockaddr *address, socklen_t size, int milliseconds) {
    int flags = fcntl(socket_fd, F_GETFL, 0);
    if (flags < 0 || fcntl(socket_fd, F_SETFL, flags | O_NONBLOCK) < 0) return 0;
    int connected = connect(socket_fd, address, size) == 0;
    if (!connected && errno == EINPROGRESS) {
        struct pollfd ready = { .fd = socket_fd, .events = POLLOUT, .revents = 0 };
        int polled;
        do polled = poll(&ready, 1, milliseconds); while (polled < 0 && errno == EINTR);
        int error = 0;
        socklen_t length = sizeof(error);
        connected = polled == 1 &&
            getsockopt(socket_fd, SOL_SOCKET, SO_ERROR, &error, &length) == 0 && error == 0;
    }
    return connected && fcntl(socket_fd, F_SETFL, flags) == 0;
}

// Line reads are served from a per-connection buffer; body reads drain it first.
struct channel { int socket; SSL_CTX *ctx; SSL *ssl; unsigned char pending[16384]; size_t start, end; };
static int send_bytes(struct channel *channel, const void *bytes, size_t size) {
    const unsigned char *cursor = bytes;
    while (size) {
        int chunk = size > 16384 ? 16384 : (int)size;
        int sent = channel->ssl ? ssl_write(channel->ssl, cursor, chunk)
                                : (int)send(channel->socket, cursor, chunk, 0);
        if (sent <= 0) return 0;
        cursor += sent;
        size -= (size_t)sent;
    }
    return 1;
}
static int read_socket(struct channel *channel, void *bytes, size_t size) {
    int count = size > 16384 ? 16384 : (int)size;
    return channel->ssl ? ssl_read(channel->ssl, bytes, count)
                        : (int)recv(channel->socket, bytes, count, 0);
}
static int receive_bytes(struct channel *channel, void *bytes, size_t size) {
    if (channel->start == channel->end) return read_socket(channel, bytes, size);
    size_t count = channel->end - channel->start;
    if (count > size) count = size;
    memcpy(bytes, channel->pending + channel->start, count);
    channel->start += count;
    return (int)count;
}
static int receive_line(struct channel *channel, char *line, size_t capacity) {
    size_t used = 0;
    while (used + 1 < capacity) {
        if (channel->start == channel->end) {
            int count = read_socket(channel, channel->pending, sizeof(channel->pending));
            if (count <= 0) return 0;
            channel->start = 0;
            channel->end = (size_t)count;
        }
        char byte = (char)channel->pending[channel->start++];
        line[used++] = byte;
        if (byte == '\n') { line[used] = 0; return 1; }
    }
    return 0;
}
static int receive_exact(struct channel *channel, void *bytes, size_t size) {
    unsigned char *cursor = bytes;
    while (size) {
        int count = receive_bytes(channel, cursor, size);
        if (count <= 0) return 0;
        cursor += count;
        size -= (size_t)count;
    }
    return 1;
}
static int copy_body(struct channel *channel, uint64_t left) {
    unsigned char block[16384];
    while (left) {
        size_t want = left > sizeof(block) ? sizeof(block) : (size_t)left;
        int count = receive_bytes(channel, block, want);
        if (count <= 0 || fwrite(block, 1, count, stdout) != (size_t)count) return 0;
        fflush(stdout);
        left -= (uint64_t)count;
    }
    return 1;
}

static int response(struct channel *channel, const char *method) {
    char linebuf[65536];
    char *headers = malloc(65536);
    if (!headers) return 0;
    int result = 0, chunked = 0, has_length = 0, status = 0;
    uint64_t length = 0;
    size_t used = 0;
    for (;;) {
        used = 0; chunked = 0; has_length = 0; length = 0;
        if (!receive_line(channel, linebuf, sizeof(linebuf)) ||
            sscanf(linebuf, "HTTP/%*s %d", &status) != 1 || status < 100 || status > 599) goto done;
        do {
            size_t n = strlen(linebuf);
            if (used + n > 65536) goto done;
            memcpy(headers + used, linebuf, n); used += n;
            if (strcmp(linebuf, "\r\n") == 0) break;
            if (strncasecmp(linebuf, "Transfer-Encoding:", 18) == 0) {
                char *value = linebuf + 18;
                while (*value == ' ' || *value == '\t') value++;
                if (strncasecmp(value, "chunked", 7) != 0) goto done;
                value += 7;
                while (*value == ' ' || *value == '\t') value++;
                if (strcmp(value, "\r\n") != 0 || chunked) goto done;
                chunked = 1;
            }
            if (strncasecmp(linebuf, "Content-Length:", 15) == 0) {
                char *value = linebuf + 15, *end = NULL;
                while (*value == ' ' || *value == '\t') value++;
                if (*value < '0' || *value > '9') goto done;
                errno = 0;
                unsigned long long parsed = strtoull(value, &end, 10);
                while (*end == ' ' || *end == '\t') end++;
                if (errno == ERANGE || strcmp(end, "\r\n") != 0 ||
                    (has_length && length != parsed)) goto done;
                length = parsed; has_length = 1;
            }
        } while (receive_line(channel, linebuf, sizeof(linebuf)));
        if (used < 4 || memcmp(headers + used - 4, "\r\n\r\n", 4) != 0 ||
            (chunked && has_length)) goto done;
        if (status < 200) {
            if (status == 101) goto done;
            continue;
        }
        break;
    }
    if (fwrite(headers, 1, used, stdout) != used) goto done;
    fflush(stdout);
    if (strcasecmp(method, "HEAD") == 0 || status == 204 || status == 304) { result = 1; goto done; }
    if (chunked) {
        for (;;) {
            if (!receive_line(channel, linebuf, sizeof(linebuf))) goto done;
            char *end = NULL;
            if (!((*linebuf >= '0' && *linebuf <= '9') ||
                  (*linebuf >= 'a' && *linebuf <= 'f') ||
                  (*linebuf >= 'A' && *linebuf <= 'F'))) goto done;
            errno = 0;
            unsigned long long chunk = strtoull(linebuf, &end, 16);
            if (errno == ERANGE) goto done;
            if (*end == ';') {
                size_t extension = strlen(end);
                if (extension < 3 || strcmp(end + extension - 2, "\r\n") != 0) goto done;
                for (size_t i = 1; i + 2 < extension; i++) {
                    if ((unsigned char)end[i] < 0x20 || (unsigned char)end[i] > 0x7e) goto done;
                }
            } else if (strcmp(end, "\r\n") != 0) goto done;
            if (chunk == 0) {
                do { if (!receive_line(channel, linebuf, sizeof(linebuf))) goto done; }
                while (strcmp(linebuf, "\r\n") != 0);
                result = 1; goto done;
            }
            if (!copy_body(channel, chunk)) goto done;
            char ending[2];
            if (!receive_exact(channel, ending, 2) || memcmp(ending, "\r\n", 2)) goto done;
        }
    }
    if (has_length) { result = copy_body(channel, length); goto done; }
    unsigned char block[16384];
    for (;;) {
        int n = receive_bytes(channel, block, sizeof(block));
        if (n == 0) { result = 1; goto done; }
        if (n < 0 || fwrite(block, 1, n, stdout) != (size_t)n) goto done;
        fflush(stdout);
    }
done:
    free(headers);
    return result;
}

int main(void) {
    int result = 1;
    const char *failure = "Native HTTP request failed";
    size_t url_len = 0, method_len = 0, body_len = 0;
    unsigned long long timeout_ms = 0, body_size = 0;
    unsigned char *url = decoded_line(&url_len), *method = decoded_line(&method_len), *body = NULL;
    struct channel channel = { .socket = -1, .ctx = NULL, .ssl = NULL };
    char *timeout = line(), *header_count = line();
    if (!url || !method || !decimal(timeout, &timeout_ms) || !header_count ||
        !valid_text(url, url_len) || !valid_text(method, method_len)) goto done;
    long count = strtol(header_count, NULL, 10);
    if (count < 0 || count > 1000) goto done;
    char **names = calloc((size_t)count + 1, sizeof(char *));
    char **values = calloc((size_t)count + 1, sizeof(char *));
    if (!names || !values) goto done;
    for (long i = 0; i < count; i++) {
        size_t n = 0, v = 0;
        names[i] = (char *)decoded_line(&n);
        values[i] = (char *)decoded_line(&v);
        if (!names[i] || !values[i] || !valid_text((unsigned char *)names[i], n) ||
            !valid_text((unsigned char *)values[i], v)) goto done_headers;
    }
    char *body_line = line();
    int body_known = decimal(body_line, &body_size) && body_size < SIZE_MAX;
    free(body_line);
    if (!body_known) goto done_headers;
    body_len = (size_t)body_size;
    body = malloc(body_len ? body_len : 1);
    if (!body || (body_len && fread(body, 1, body_len, stdin) != body_len)) goto done_headers;
    int secure = strncmp((char *)url, "https://", 8) == 0;
    char *authority = (char *)url + (secure ? 8 : 7);
    if (!secure && strncmp((char *)url, "http://", 7)) goto done_headers;
    char *path = strpbrk(authority, "/?#");
    char *target = path ? path : "/";
    size_t authority_len = path ? (size_t)(path - authority) : strlen(authority);
    char *hostport = strndup(authority, authority_len);
    if (!hostport || !authority_len || strchr(hostport, '@')) { free(hostport); goto done_headers; }
    char *host = hostport, *port = secure ? "443" : "80";
    if (host[0] == '[') {
        char *close = strchr(host, ']');
        if (!close) { free(hostport); goto done_headers; }
        *close = 0; host++;
        if (close[1] == ':') port = close + 2;
    } else {
        char *colon = strrchr(host, ':');
        if (colon) { *colon = 0; port = colon + 1; }
    }
    char *proxy_hostport = NULL;
    char *connect_host = host, *connect_port = port;
    int proxy = 0;
    const char *proxy_url = getenv(secure ? "HTTPS_PROXY" : "HTTP_PROXY");
    if (!proxy_url) proxy_url = getenv(secure ? "https_proxy" : "http_proxy");
    if (proxy_url && *proxy_url &&
        strcmp(host, "localhost") && strcmp(host, "127.0.0.1") && strcmp(host, "::1")) {
        const char *no_proxy = getenv("NO_PROXY");
        if (!no_proxy) no_proxy = getenv("no_proxy");
        int bypass = 0;
        if (no_proxy) {
            char *list = strdup(no_proxy), *save = NULL;
            if (list) {
                for (char *entry = strtok_r(list, ",", &save); entry; entry = strtok_r(NULL, ",", &save)) {
                    while (*entry == ' ') entry++;
                    if (!strcmp(entry, "*") || !strcasecmp(entry, host)) bypass = 1;
                    size_t host_len = strlen(host), entry_len = strlen(entry);
                    if (entry_len && entry[0] == '.' && host_len > entry_len &&
                        !strcasecmp(host + host_len - entry_len, entry)) bypass = 1;
                }
                free(list);
            }
        }
        if (!bypass) {
            const char *origin = proxy_url;
            if (strstr(proxy_url, "://")) {
                if (strncasecmp(proxy_url, "http://", 7)) {
                    failure = "Native HTTP supports only http:// proxies; unset or change HTTP(S)_PROXY";
                    free(hostport); goto done_headers;
                }
                origin += 7;
            }
            size_t length = strcspn(origin, "/");
            proxy_hostport = strndup(origin, length);
            if (!proxy_hostport || strchr(proxy_hostport, '@')) { free(hostport); free(proxy_hostport); goto done_headers; }
            connect_host = proxy_hostport;
            char *colon = strrchr(proxy_hostport, ':');
            if (colon) { *colon = 0; connect_port = colon + 1; }
            else connect_port = "80";
            proxy = 1;
        }
    }
    struct addrinfo hints = {0}, *addresses = NULL;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(connect_host, connect_port, &hints, &addresses)) { free(hostport); free(proxy_hostport); goto done_headers; }
    int attempt = timeout_ms && timeout_ms < 10000 ? (int)timeout_ms : 10000;
    for (struct addrinfo *address = addresses; address; address = address->ai_next) {
        channel.socket = socket(address->ai_family, address->ai_socktype, address->ai_protocol);
        if (channel.socket >= 0 &&
            connect_within(channel.socket, address->ai_addr, address->ai_addrlen, attempt)) break;
        if (channel.socket >= 0) close(channel.socket);
        channel.socket = -1;
    }
    freeaddrinfo(addresses);
    if (channel.socket < 0) { free(hostport); free(proxy_hostport); goto done_headers; }
    if (timeout_ms) {
        // Matches WinHTTP: each send or receive waits at most the timeout, capped at 120 s.
        unsigned long long limit = timeout_ms < 120000 ? timeout_ms : 120000;
        struct timeval wait = { .tv_sec = (time_t)(limit / 1000), .tv_usec = (suseconds_t)(limit % 1000 * 1000) };
        if (setsockopt(channel.socket, SOL_SOCKET, SO_RCVTIMEO, &wait, sizeof(wait)) ||
            setsockopt(channel.socket, SOL_SOCKET, SO_SNDTIMEO, &wait, sizeof(wait))) {
            free(hostport); free(proxy_hostport); goto done_headers;
        }
    }
    if (secure && proxy) {
        char connect_request[2048], reply[65536];
        if (snprintf(connect_request, sizeof(connect_request),
            "CONNECT %s:%s HTTP/1.1\r\nHost: %s:%s\r\n\r\n", host, port, host, port) >= (int)sizeof(connect_request) ||
            !send_bytes(&channel, connect_request, strlen(connect_request)) ||
            !receive_line(&channel, reply, sizeof(reply))) { free(hostport); free(proxy_hostport); goto done_headers; }
        int proxy_status = 0;
        if (sscanf(reply, "HTTP/%*s %d", &proxy_status) != 1 || proxy_status != 200) {
            free(hostport); free(proxy_hostport); goto done_headers;
        }
        do { if (!receive_line(&channel, reply, sizeof(reply))) { free(hostport); free(proxy_hostport); goto done_headers; } }
        while (strcmp(reply, "\r\n") != 0);
        // TLS starts on a clean tunnel; buffered bytes past the reply are unexpected.
        if (channel.start != channel.end) { free(hostport); free(proxy_hostport); goto done_headers; }
    }
    if (secure) {
        if (!load_tls()) { free(hostport); free(proxy_hostport); goto done_headers; }
        channel.ctx = ssl_ctx_new(tls_client_method());
        if (!channel.ctx) { free(hostport); free(proxy_hostport); goto done_headers; }
        ssl_ctx_verify(channel.ctx, 1, NULL);
        if (ssl_ctx_paths(channel.ctx) != 1) { free(hostport); free(proxy_hostport); goto done_headers; }
        channel.ssl = ssl_new(channel.ctx);
        unsigned char ip[16];
        int ip_literal = inet_pton(AF_INET, host, ip) == 1 || inet_pton(AF_INET6, host, ip) == 1;
        int identity_set = channel.ssl && (ip_literal
            ? verify_ip(ssl_get0_param(channel.ssl), host)
            : ssl_set1_host(channel.ssl, host));
        if (!channel.ssl || ssl_set_fd(channel.ssl, channel.socket) != 1 ||
            (!ip_literal && ssl_ctrl(channel.ssl, 55, 0, host) != 1) || identity_set != 1 ||
            ssl_connect(channel.ssl) != 1 || ssl_verify_result(channel.ssl) != 0) {
            free(hostport); free(proxy_hostport); goto done_headers;
        }
    }
    char start[4096];
    const char *target_path = *target == '?' ? "/" : "";
    if (snprintf(start, sizeof(start), "%s %s%s HTTP/1.1\r\nHost: %.*s\r\nConnection: close\r\n",
        method, proxy && !secure ? (char *)url : target_path, proxy && !secure ? "" : target,
        (int)authority_len, authority) >= (int)sizeof(start) ||
        !send_bytes(&channel, start, strlen(start))) { free(hostport); free(proxy_hostport); goto done_headers; }
    int content_length = 0;
    for (long i = 0; i < count; i++) {
        if (strcasecmp(names[i], "Content-Length") == 0) content_length = 1;
        if (!send_bytes(&channel, names[i], strlen(names[i])) ||
            !send_bytes(&channel, ": ", 2) ||
            !send_bytes(&channel, values[i], strlen(values[i])) ||
            !send_bytes(&channel, "\r\n", 2)) { free(hostport); free(proxy_hostport); goto done_headers; }
    }
    if (!content_length && (body_len || strcasecmp((char *)method, "POST") == 0 ||
        strcasecmp((char *)method, "PUT") == 0 || strcasecmp((char *)method, "PATCH") == 0)) {
        char content[80];
        snprintf(content, sizeof(content), "Content-Length: %zu\r\n", body_len);
        if (!send_bytes(&channel, content, strlen(content))) { free(hostport); free(proxy_hostport); goto done_headers; }
    }
    if (!send_bytes(&channel, "\r\n", 2) ||
        (body_len && !send_bytes(&channel, body, body_len))) { free(hostport); free(proxy_hostport); goto done_headers; }
    free(hostport);
    free(proxy_hostport);
    if (response(&channel, (char *)method)) result = 0;
done_headers:
    for (long i = 0; i < count; i++) { free(names[i]); free(values[i]); }
    free(names); free(values);
done:
    if (channel.ssl) ssl_free(channel.ssl);
    if (channel.ctx) ssl_ctx_free(channel.ctx);
    if (channel.socket >= 0) close(channel.socket);
    free(url); free(method); free(body); free(timeout); free(header_count);
    if (result) fprintf(stderr, "%s\n", failure);
    return result;
}
