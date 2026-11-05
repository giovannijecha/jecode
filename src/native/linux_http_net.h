// Connections for the Linux HTTP adapter: OS sockets, the OS-supplied OpenSSL TLS
// ABI and a small cache of open connections. A connection is opened through the
// proxy variables once, then reused until it closes or a response ends it.
#include <arpa/inet.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/time.h>

typedef struct ssl_ctx_st SSL_CTX;
typedef struct ssl_st SSL;
typedef struct ssl_method_st SSL_METHOD;
typedef struct X509_VERIFY_PARAM_st X509_VERIFY_PARAM;
static const SSL_METHOD *(*tls_client_method)(void);
static SSL_CTX *(*ssl_ctx_new)(const SSL_METHOD *);
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
static int (*ssl_get_error)(const SSL *, int);
static long (*ssl_verify_result)(const SSL *);
static unsigned long (*err_peek_last_error)(void);
static void (*err_clear_error)(void);
static int (*ssl_write)(SSL *, const void *, int);
static int (*ssl_read)(SSL *, void *, int);
static int (*ssl_pending)(const SSL *);
static unsigned char *(*sha1_digest)(const unsigned char *, size_t, unsigned char *);
static int tls3;

static int load_tls(void) {
    static int loaded;
    if (loaded) return 1;
    void *library = dlopen("libssl.so.3", RTLD_NOW | RTLD_LOCAL);
    tls3 = library != NULL;
    if (!library) library = dlopen("libssl.so.1.1", RTLD_NOW | RTLD_LOCAL);
    if (!library) return 0;
#define SYMBOL(slot, name) do { *(void **)(&slot) = dlsym(library, name); if (!slot) return 0; } while (0)
    SYMBOL(tls_client_method, "TLS_client_method");
    SYMBOL(ssl_ctx_new, "SSL_CTX_new");
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
    SYMBOL(ssl_get_error, "SSL_get_error");
    SYMBOL(ssl_verify_result, "SSL_get_verify_result");
    SYMBOL(err_clear_error, "ERR_clear_error");
    if (tls3) *(void **)(&err_peek_last_error) = dlsym(library, "ERR_peek_last_error");
    SYMBOL(ssl_write, "SSL_write");
    SYMBOL(ssl_read, "SSL_read");
    SYMBOL(ssl_pending, "SSL_pending");
    /* In libcrypto, which libssl loads; only the WebSocket handshake needs it. */
    *(void **)(&sha1_digest) = dlsym(library, "SHA1");
#undef SYMBOL
    loaded = 1;
    return 1;
}

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

// Line reads are served from a per-connection buffer; body reads drain it first.
struct channel {
    int socket;
    SSL *ssl;
    char *key;
    unsigned char pending[16384];
    size_t start, end;
};

static void close_channel(struct channel *channel) {
    if (!channel) return;
    if (channel->ssl) ssl_free(channel->ssl);
    if (channel->socket >= 0) close(channel->socket);
    free(channel->key);
    free(channel);
}

// Keeps the owner's exit visible during a long response: closing stdin ends the
// helper. Only a hangup is asked for, because a queued request is not its concern.
static int wait_readable(struct channel *channel) {
    if (channel->ssl && ssl_pending(channel->ssl) > 0) return 1;
    struct pollfd ready[2] = { { channel->socket, POLLIN, 0 }, { STDIN_FILENO, 0, 0 } };
    int polled;
    do polled = poll(ready, 2, operation_limit_ms); while (polled < 0 && errno == EINTR);
    if (polled == 0) return fail('T', "Native HTTP request timed out");
    if (polled < 0) return fail('T', "Native HTTP wait failed");
    if (ready[1].revents & (POLLHUP | POLLERR | POLLNVAL)) _exit(0);
    return 1;
}

static int send_bytes(struct channel *channel, const void *bytes, size_t size) {
    const unsigned char *cursor = bytes;
    while (size) {
        int chunk = size > 65536 ? 65536 : (int)size;
        int sent = channel->ssl ? ssl_write(channel->ssl, cursor, chunk)
                                : (int)send(channel->socket, cursor, (size_t)chunk, MSG_NOSIGNAL);
        if (sent <= 0) return fail('T', "Native HTTP send failed");
        cursor += sent;
        size -= (size_t)sent;
    }
    return 1;
}

static int read_socket(struct channel *channel, void *bytes, size_t size) {
    if (!wait_readable(channel)) return -1;
    int count = size > 16384 ? 16384 : (int)size;
    return channel->ssl ? ssl_read(channel->ssl, bytes, count)
                        : (int)recv(channel->socket, bytes, (size_t)count, 0);
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

// Where a request goes and how to reach it. The key names a reusable connection.
struct target {
    int secure, proxy;
    const char *authority, *path;
    size_t authority_length;
    char *hostport, *host, *port;
    char *proxy_hostport, *connect_host, *connect_port;
    char key[768];
};

static void free_target(struct target *target) {
    free(target->hostport);
    free(target->proxy_hostport);
}

static int proxy_bypassed(const char *host) {
    const char *no_proxy = getenv("NO_PROXY");
    if (!no_proxy) no_proxy = getenv("no_proxy");
    char *list = no_proxy ? strdup(no_proxy) : NULL;
    int bypass = 0;
    if (list) {
        char *save = NULL;
        for (char *entry = strtok_r(list, ",", &save); entry; entry = strtok_r(NULL, ",", &save)) {
            while (*entry == ' ') entry++;
            if (!strcmp(entry, "*") || !strcasecmp(entry, host)) bypass = 1;
            size_t host_length = strlen(host), entry_length = strlen(entry);
            if (entry_length && entry[0] == '.' && host_length > entry_length &&
                !strcasecmp(host + host_length - entry_length, entry)) bypass = 1;
        }
        free(list);
    }
    return bypass;
}

static int parse_target(const char *url, struct target *target) {
    memset(target, 0, sizeof(*target));
    target->secure = strncmp(url, "https://", 8) == 0;
    if (!target->secure && strncmp(url, "http://", 7)) return fail('E', "Native HTTP URL is invalid");
    target->authority = url + (target->secure ? 8 : 7);
    target->path = strpbrk(target->authority, "/?#");
    target->authority_length = target->path ? (size_t)(target->path - target->authority) : strlen(target->authority);
    if (!target->path) target->path = "/";
    target->hostport = strndup(target->authority, target->authority_length);
    if (!target->hostport || !target->authority_length || strchr(target->hostport, '@'))
        return fail('E', "Native HTTP URL is invalid");
    target->host = target->hostport;
    target->port = target->secure ? "443" : "80";
    if (target->host[0] == '[') {
        char *close_bracket = strchr(target->host, ']');
        if (!close_bracket) return fail('E', "Native HTTP URL is invalid");
        *close_bracket = 0;
        target->host++;
        if (close_bracket[1] == ':') target->port = close_bracket + 2;
    } else {
        char *colon = strrchr(target->host, ':');
        if (colon) { *colon = 0; target->port = colon + 1; }
    }
    target->connect_host = target->host;
    target->connect_port = target->port;
    const char *proxy_url = getenv(target->secure ? "HTTPS_PROXY" : "HTTP_PROXY");
    if (!proxy_url) proxy_url = getenv(target->secure ? "https_proxy" : "http_proxy");
    if (proxy_url && *proxy_url && strcmp(target->host, "localhost") && strcmp(target->host, "127.0.0.1") &&
        strcmp(target->host, "::1") && !proxy_bypassed(target->host)) {
        const char *origin = proxy_url;
        if (strstr(proxy_url, "://")) {
            if (strncasecmp(proxy_url, "http://", 7))
                return fail('E', "Native HTTP supports only http:// proxies; unset or change HTTP(S)_PROXY");
            origin += 7;
        }
        target->proxy_hostport = strndup(origin, strcspn(origin, "/"));
        if (!target->proxy_hostport || strchr(target->proxy_hostport, '@'))
            return fail('E', "Native HTTP proxy address is invalid");
        target->connect_host = target->proxy_hostport;
        char *colon = strrchr(target->proxy_hostport, ':');
        if (colon) { *colon = 0; target->connect_port = colon + 1; }
        else target->connect_port = "80";
        target->proxy = 1;
    }
    if (snprintf(target->key, sizeof(target->key), "%d|%s|%s", target->secure, target->host, target->port) >= (int)sizeof(target->key))
        return fail('E', "Native HTTP URL is invalid");
    return 1;
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

static void set_socket_limits(struct channel *channel, int milliseconds) {
    struct timeval wait = { .tv_sec = milliseconds / 1000, .tv_usec = (milliseconds % 1000) * 1000 };
    (void)setsockopt(channel->socket, SOL_SOCKET, SO_RCVTIMEO, &wait, sizeof(wait));
    (void)setsockopt(channel->socket, SOL_SOCKET, SO_SNDTIMEO, &wait, sizeof(wait));
}

static int open_tunnel(struct channel *channel, const struct target *target) {
    char request[2048], reply[65536];
    int length = snprintf(request, sizeof(request), "CONNECT %s:%s HTTP/1.1\r\nHost: %s:%s\r\n\r\n",
        target->host, target->port, target->host, target->port);
    if (length < 0 || length >= (int)sizeof(request)) return fail('E', "Native HTTP URL is invalid");
    if (!send_bytes(channel, request, (size_t)length) || !receive_line(channel, reply, sizeof(reply)))
        return fail('T', "Native HTTP proxy connection failed");
    int status = 0;
    if (sscanf(reply, "HTTP/%*s %d", &status) != 1 || status != 200) {
        int temporary = status == 408 || status == 425 || status == 429 || (status >= 500 && status <= 599);
        return fail(temporary ? 'T' : 'E', "Native HTTP proxy refused the connection");
    }
    do {
        if (!receive_line(channel, reply, sizeof(reply))) return fail('T', "Native HTTP proxy connection failed");
    } while (strcmp(reply, "\r\n") != 0);
    // TLS starts on a clean tunnel; buffered bytes past the reply are unexpected.
    if (channel->start != channel->end) return fail('E', "Native HTTP proxy sent unexpected data");
    return 1;
}

static int open_tls(struct channel *channel, const char *host) {
    static SSL_CTX *context;
    if (!load_tls()) return fail('E', "Native HTTP needs the system OpenSSL library (libssl)");
    if (!context) {
        SSL_CTX *created = ssl_ctx_new(tls_client_method());
        if (!created) return fail('E', "Native HTTP could not initialize TLS");
        ssl_ctx_verify(created, 1, NULL);
        if (ssl_ctx_paths(created) != 1) return fail('E', "Native HTTP could not load system certificates");
        context = created;
    }
    channel->ssl = ssl_new(context);
    unsigned char ip[16];
    int ip_literal = inet_pton(AF_INET, host, ip) == 1 || inet_pton(AF_INET6, host, ip) == 1;
    int identity_set = channel->ssl && (ip_literal
        ? verify_ip(ssl_get0_param(channel->ssl), host)
        : ssl_set1_host(channel->ssl, host));
    if (!channel->ssl || ssl_set_fd(channel->ssl, channel->socket) != 1 ||
        (!ip_literal && ssl_ctrl(channel->ssl, 55, 0, (void *)host) != 1) || identity_set != 1)
        return fail('E', "Native HTTP could not initialize TLS");
    err_clear_error();
    int handshake = ssl_connect(channel->ssl);
    if (handshake != 1) {
        int error = ssl_get_error(channel->ssl, handshake);
        // OpenSSL 3 reports a peer EOF as SSL_ERROR_SSL with reason 294.
        unsigned long detail = tls3 && err_peek_last_error ? err_peek_last_error() : 0;
        int peer_eof = tls3 && ((detail >> 23) & 0xff) == 20 && (detail & 0x7fffff) == 294;
        int temporary = ssl_verify_result(channel->ssl) == 0 &&
            (error == 2 || error == 3 || error == 5 || error == 6 || peer_eof);
        return fail(temporary ? 'T' : 'E', "Native HTTP TLS connection failed");
    }
    if (ssl_verify_result(channel->ssl) != 0) return fail('E', "Native HTTP TLS certificate was rejected");
    return 1;
}

static struct channel *open_channel(const struct target *target, int connect_ms) {
    struct addrinfo hints = {0}, *addresses = NULL;
    hints.ai_socktype = SOCK_STREAM;
    int lookup = getaddrinfo(target->connect_host, target->connect_port, &hints, &addresses);
    if (lookup) {
        fail(lookup == EAI_AGAIN ? 'T' : 'E', "Native HTTP could not resolve the host");
        return NULL;
    }
    struct channel *channel = calloc(1, sizeof(*channel));
    if (!channel) { freeaddrinfo(addresses); fail('E', "Native HTTP is out of memory"); return NULL; }
    channel->socket = -1;
    for (struct addrinfo *address = addresses; address; address = address->ai_next) {
        channel->socket = socket(address->ai_family, address->ai_socktype, address->ai_protocol);
        if (channel->socket >= 0 && connect_within(channel->socket, address->ai_addr, address->ai_addrlen, connect_ms)) break;
        if (channel->socket >= 0) close(channel->socket);
        channel->socket = -1;
    }
    freeaddrinfo(addresses);
    if (channel->socket < 0) {
        close_channel(channel);
        fail('T', "Native HTTP could not connect");
        return NULL;
    }
    // A request is one write; waiting for an earlier acknowledgment only adds delay.
    int enabled = 1;
    (void)setsockopt(channel->socket, IPPROTO_TCP, TCP_NODELAY, &enabled, sizeof(enabled));
    channel->key = strdup(target->key);
    set_socket_limits(channel, connect_ms > 30000 ? connect_ms : 30000);
    if (!channel->key || (target->secure && target->proxy && !open_tunnel(channel, target)) ||
        (target->secure && !open_tls(channel, target->host))) {
        close_channel(channel);
        return NULL;
    }
    return channel;
}

// A small cache: an adapter serves one request at a time, usually to one host.
enum { CACHED_CHANNELS = 4 };
static struct channel *cached[CACHED_CHANNELS];

// Whether an idle connection can still carry a request. A TLS session ticket is
// expected on an idle connection; anything else readable means it is ending.
static int channel_idle_usable(struct channel *channel) {
    if (channel->start != channel->end) return 0;
    struct pollfd ready = { channel->socket, POLLIN, 0 };
    int polled = poll(&ready, 1, 0);
    if (polled == 0) return 1;
    if (polled < 0 || (ready.revents & (POLLERR | POLLHUP | POLLNVAL)) || !channel->ssl) return 0;
    int flags = fcntl(channel->socket, F_GETFL, 0);
    if (flags < 0 || fcntl(channel->socket, F_SETFL, flags | O_NONBLOCK) < 0) return 0;
    unsigned char ignored[256];
    err_clear_error();
    int count = ssl_read(channel->ssl, ignored, sizeof(ignored));
    int blocked = count <= 0 && ssl_get_error(channel->ssl, count) == 2;
    return fcntl(channel->socket, F_SETFL, flags) == 0 && blocked;
}

static struct channel *take_cached(const char *key) {
    for (int i = 0; i < CACHED_CHANNELS; i++) {
        struct channel *channel = cached[i];
        if (!channel || strcmp(channel->key, key)) continue;
        cached[i] = NULL;
        if (channel_idle_usable(channel)) return channel;
        close_channel(channel);
    }
    return NULL;
}

static void keep_channel(struct channel *channel) {
    for (int i = 0; i < CACHED_CHANNELS; i++) {
        if (!cached[i]) { cached[i] = channel; return; }
    }
    close_channel(cached[0]);
    memmove(cached, cached + 1, sizeof(cached[0]) * (CACHED_CHANNELS - 1));
    cached[CACHED_CHANNELS - 1] = channel;
}
