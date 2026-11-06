// Unix OS sockets, platform TLS and a small cache of open connections. A connection is opened through the
// proxy variables once, then reused until it closes or a response ends it.
#include <arpa/inet.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <time.h>
#ifndef MSG_NOSIGNAL
#define MSG_NOSIGNAL 0
#endif

// Line reads are served from a per-connection buffer; body reads drain it first.
struct channel {
    int socket;
    void *ssl;
    char *key;
    unsigned char pending[16384];
    size_t start, end;
    time_t kept_at;
};

#ifdef __APPLE__
#include "macos_http_tls.h"
#else
#include "linux_http_tls.h"
#endif
static void close_channel(struct channel *channel) {
    if (!channel) return;
    if (channel->ssl) tls_free(channel);
    if (channel->socket >= 0) close(channel->socket);
    free(channel->key);
    free(channel);
}

// Keeps the owner's exit visible during a long response: closing stdin ends the
// helper. Only a hangup is asked for, because a queued request is not its concern.
static int wait_readable(struct channel *channel) {
    if (channel->ssl && tls_pending(channel) > 0) return 1;
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
        int sent = channel->ssl ? tls_write(channel, cursor, chunk)
                                : (int)send(channel->socket, cursor, (size_t)chunk, MSG_NOSIGNAL);
        if (sent <= 0) return channel->ssl ? 0 : fail('T', "Native HTTP send failed");
        cursor += sent;
        size -= (size_t)sent;
    }
    return 1;
}

static int read_socket(struct channel *channel, void *bytes, size_t size) {
    if (!wait_readable(channel)) return -1;
    int count = size > 16384 ? 16384 : (int)size;
    return channel->ssl ? tls_read(channel, bytes, count)
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
#ifdef __APPLE__
    int socks;
    char *proxy_user, *proxy_password, *proxy_authorization;
    CFArrayRef proxy_routes;
    CFIndex proxy_index;
#endif
    const char *authority, *path;
    size_t authority_length;
    char *hostport, *host, *port;
    char *proxy_hostport, *connect_host, *connect_port;
    char key[768];
};

static void free_target(struct target *target) {
    free(target->hostport);
    free(target->proxy_hostport);
#ifdef __APPLE__
    free(target->proxy_user);
    free(target->proxy_password);
    free(target->proxy_authorization);
    if (target->proxy_routes) CFRelease(target->proxy_routes);
#endif
}

static int loopback_host(const char *host) {
    return !strcasecmp(host, "localhost") || !strncmp(host, "127.", 4) || !strcmp(host, "::1");
}

#ifdef __APPLE__
#include "macos_http_proxy.h"
#else
// `example.com`, `.example.com` and `*.example.com` all cover the domain's hosts.
static int proxy_bypassed(const char *host) {
    const char *no_proxy = getenv("NO_PROXY");
    if (!no_proxy) no_proxy = getenv("no_proxy");
    char *list = no_proxy ? strdup(no_proxy) : NULL;
    int bypass = 0;
    if (list) {
        char *save = NULL;
        size_t host_length = strlen(host);
        for (char *entry = strtok_r(list, ",", &save); entry; entry = strtok_r(NULL, ",", &save)) {
            while (*entry == ' ' || *entry == '\t') entry++;
            size_t length = strlen(entry);
            while (length && (entry[length - 1] == ' ' || entry[length - 1] == '\t')) entry[--length] = 0;
            if (!strcmp(entry, "*")) bypass = 1;
            if (entry[0] == '*' && entry[1] == '.') { entry++; length--; }
            if (entry[0] == '.') { entry++; length--; }
            if (!length) continue;
            if (!strcasecmp(entry, host) ||
                (host_length > length && host[host_length - length - 1] == '.' &&
                 !strcasecmp(host + host_length - length, entry))) bypass = 1;
        }
        free(list);
    }
    return bypass;
}
#endif

static int target_key(struct target *target) {
    int proxy_kind = target->proxy;
#ifdef __APPLE__
    if (target->socks) proxy_kind = 2;
#endif
    if (snprintf(target->key, sizeof(target->key), "%d|%s|%s|%d|%s|%s", target->secure,
        target->host, target->port, proxy_kind, target->connect_host, target->connect_port) >= (int)sizeof(target->key))
        return fail('E', "Native HTTP URL is invalid");
    return 1;
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
    if (loopback_host(target->host)) return target_key(target);
#ifdef __APPLE__
    // macOS follows the system settings alone; the WebSocket uses them too.
    if (!system_proxy(target, url)) return 0;
#else
    const char *proxy_url = getenv(target->secure ? "HTTPS_PROXY" : "HTTP_PROXY");
    if (!proxy_url) proxy_url = getenv(target->secure ? "https_proxy" : "http_proxy");
    if (proxy_url && *proxy_url && !proxy_bypassed(target->host)) {
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
        target->connect_port = "80";
        char *separator = target->proxy_hostport;
        if (*separator == '[') {
            char *end = strchr(separator, ']');
            if (!end || (end[1] && end[1] != ':')) return fail('E', "Native HTTP proxy address is invalid");
            *end = 0;
            target->connect_host = separator + 1;
            separator = end[1] == ':' ? end + 1 : NULL;
        } else {
            separator = strrchr(separator, ':');
        }
        if (separator) { *separator = 0; target->connect_port = separator + 1; }
        target->proxy = 1;
    }
#endif
    return target_key(target);
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
    int literal = strchr(target->host, ':') != NULL; // An IPv6 address needs its brackets back.
    const char *left = literal ? "[" : "", *right = literal ? "]" : "";
    int length = snprintf(request, sizeof(request), "CONNECT %s%s%s:%s HTTP/1.1\r\nHost: %s%s%s:%s\r\n",
        left, target->host, right, target->port, left, target->host, right, target->port);
#ifdef __APPLE__
    if (length > 0 && length < (int)sizeof(request) && target->proxy_authorization)
        length += snprintf(request + length, sizeof(request) - (size_t)length,
            "Proxy-Authorization: %s\r\n", target->proxy_authorization);
#endif
    if (length > 0 && length < (int)sizeof(request))
        length += snprintf(request + length, sizeof(request) - (size_t)length, "\r\n");
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

static struct channel *open_channel_once(const struct target *target, int connect_ms) {
    struct addrinfo hints = {0}, *addresses = NULL;
    hints.ai_socktype = SOCK_STREAM;
    int lookup = getaddrinfo(target->connect_host, target->connect_port, &hints, &addresses);
    if (lookup) {
        // Without a network macOS answers EAI_NONAME, so an unknown name may be temporary too.
        fail(lookup == EAI_AGAIN || lookup == EAI_NONAME ? 'T' : 'E', "Native HTTP could not resolve the host");
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
#ifdef __APPLE__
    if (target->socks && !open_socks(channel, target)) { close_channel(channel); return NULL; }
#endif
    if (!channel->key || (target->secure && target->proxy && !open_tunnel(channel, target)) ||
        (target->secure && !open_tls(channel, target->host))) {
        close_channel(channel);
        return NULL;
    }
    return channel;
}

// A small cache: an adapter serves one request at a time, usually to one host.
enum { CACHED_CHANNELS = 4, CHANNEL_IDLE_SECONDS = 90 };
static struct channel *cached[CACHED_CHANNELS];

// Whether an idle connection can still carry a request. A TLS session ticket is
// expected on an idle connection; anything else readable means it is ending. A
// connection silent for long is not trusted: after a computer sleeps no close ever
// arrives, and a request sent on it would wait out the whole receive limit.
static int channel_idle_usable(struct channel *channel) {
    if (channel->start != channel->end || (channel->ssl && tls_pending(channel) > 0)) return 0;
    if (difftime(time(NULL), channel->kept_at) > CHANNEL_IDLE_SECONDS) return 0;
    struct pollfd ready = { channel->socket, POLLIN, 0 };
    int polled = poll(&ready, 1, 0);
    if (polled == 0) return 1;
    if (polled < 0 || (ready.revents & (POLLERR | POLLHUP | POLLNVAL)) || !channel->ssl) return 0;
    return tls_idle_usable(channel);
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
    channel->kept_at = time(NULL);
    for (int i = 0; i < CACHED_CHANNELS; i++) {
        if (!cached[i]) { cached[i] = channel; return; }
    }
    close_channel(cached[0]);
    memmove(cached, cached + 1, sizeof(cached[0]) * (CACHED_CHANNELS - 1));
    cached[CACHED_CHANNELS - 1] = channel;
}

static struct channel *open_channel(struct target *target, int connect_ms) {
    for (;;) {
        struct channel *channel = open_channel_once(target, connect_ms);
        if (channel) return channel;
#ifdef __APPLE__
        if (!next_system_proxy(target) || !target_key(target)) return NULL;
        channel = take_cached(target->key);
        if (channel) return channel;
#else
        return NULL;
#endif
    }
}
