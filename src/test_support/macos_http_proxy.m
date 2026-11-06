#import <Foundation/Foundation.h>
#import <CFNetwork/CFNetwork.h>
#define main jecode_http_entry
#include "../native/macos_http.m"
#undef main

#define CHECK(condition) do { if (!(condition)) { \
    fprintf(stderr, "%s:%d: %s\n", __func__, __LINE__, #condition); return 0; \
} } while (0)

static int read_fully(int fd, void *buffer, size_t size) {
    unsigned char *cursor = buffer;
    while (size) {
        ssize_t count = recv(fd, cursor, size, 0);
        if (count <= 0) return 0;
        cursor += count;
        size -= (size_t)count;
    }
    return 1;
}

static int fixture_read_head(int fd, char *buffer, size_t capacity) {
    size_t used = 0;
    while (used + 1 < capacity) {
        if (recv(fd, buffer + used, 1, 0) != 1) return 0;
        buffer[++used] = 0;
        if (used >= 4 && !strcmp(buffer + used - 4, "\r\n\r\n")) return 1;
    }
    return 0;
}

static void socket_limit(int fd) {
    struct timeval limit = { .tv_sec = 5 };
    (void)setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &limit, sizeof(limit));
    (void)setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &limit, sizeof(limit));
}

static CFURLRef fixture_url(void) {
    return CFURLCreateWithString(NULL, CFSTR("http://origin.test/resource"), NULL);
}

static CFArrayRef scripted_routes(const char *script) {
    CFURLRef url = fixture_url();
    CFStringRef javascript = CFStringCreateWithCString(NULL, script, kCFStringEncodingUTF8);
    NSDictionary *entry = @{
        (__bridge NSString *)kCFProxyTypeKey: (__bridge NSString *)kCFProxyTypeAutoConfigurationJavaScript,
        (__bridge NSString *)kCFProxyAutoConfigurationJavaScriptKey: (__bridge NSString *)javascript
    };
    CFMutableArrayRef routes = CFArrayCreateMutable(NULL, 0, &kCFTypeArrayCallBacks);
    proxy_routes(routes, (__bridge CFArrayRef)@[entry], url, 0);
    CFRelease(javascript);
    CFRelease(url);
    return routes;
}

static int pac_order(void) {
    CHECK(!loopback_host("127.example.test"));
    CHECK(loopback_host("127.0.0.2"));
    CFArrayRef routes = scripted_routes("function FindProxyForURL(url, host) { return 'PROXY 127.0.0.1:12345; DIRECT'; }");
    CHECK(routes && CFArrayGetCount(routes) == 2);
    CFDictionaryRef first = CFArrayGetValueAtIndex(routes, 0);
    CFDictionaryRef second = CFArrayGetValueAtIndex(routes, 1);
    CHECK(CFEqual(CFDictionaryGetValue(first, kCFProxyTypeKey), kCFProxyTypeHTTP));
    CHECK(CFEqual(CFDictionaryGetValue(second, kCFProxyTypeKey), kCFProxyTypeNone));
    struct target target = {0};
    target.host = "origin.test"; target.port = "80";
    target.proxy_routes = routes;
    CHECK(next_system_proxy(&target) && target.proxy && target.proxy_index == 1);
    CHECK(!strcmp(target.connect_host, "127.0.0.1") && !strcmp(target.connect_port, "12345"));
    CHECK(next_system_proxy(&target) && !target.proxy && target.proxy_index == 2);
    free_target(&target);
    return 1;
}

static int listener(unsigned short *port, int listening) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return -1;
    struct sockaddr_in address = {0};
    address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(fd, (struct sockaddr *)&address, sizeof(address)) || (listening && listen(fd, 1))) {
        close(fd); return -1;
    }
    socklen_t size = sizeof(address);
    if (getsockname(fd, (struct sockaddr *)&address, &size)) { close(fd); return -1; }
    *port = ntohs(address.sin_port);
    return fd;
}

static int fallback_route(void) {
    unsigned short blocked_port, origin_port;
    int blocked = listener(&blocked_port, 0);
    int origin = listener(&origin_port, 1);
    CHECK(blocked >= 0 && origin >= 0);
    char script[180], port[8];
    snprintf(script, sizeof(script), "function FindProxyForURL(url, host) { return 'PROXY 127.0.0.1:%u; DIRECT'; }", blocked_port);
    snprintf(port, sizeof(port), "%u", origin_port);
    struct target target = {0};
    target.hostport = strdup("127.0.0.1"); target.host = target.hostport; target.port = port;
    target.proxy_routes = scripted_routes(script);
    CHECK(target.host && target.proxy_routes && next_system_proxy(&target) && target_key(&target));
    struct channel *channel = open_channel(&target, 1000);
    CHECK(channel && !target.proxy && target.proxy_index == 2);
    struct pollfd ready = { .fd = origin, .events = POLLIN };
    CHECK(poll(&ready, 1, 1000) == 1 && (ready.revents & POLLIN));
    int accepted = accept(origin, NULL, NULL);
    CHECK(accepted >= 0);
    close(accepted); close_channel(channel); close(origin); close(blocked);
    free_target(&target);
    return 1;
}

static CFArrayRef authenticated_route(void) {
    NSDictionary *route = @{
        (__bridge NSString *)kCFProxyTypeKey: (__bridge NSString *)kCFProxyTypeHTTP,
        (__bridge NSString *)kCFProxyHostNameKey: @"127.0.0.1",
        (__bridge NSString *)kCFProxyPortNumberKey: @8080,
        (__bridge NSString *)kCFProxyUsernameKey: @"user",
        (__bridge NSString *)kCFProxyPasswordKey: @"pass"
    };
    return (__bridge_retained CFArrayRef)@[route];
}

struct head_peer { int fd, ok; char text[2048]; };
static void *tunnel_peer(void *opaque) {
    struct head_peer *peer = opaque;
    peer->ok = fixture_read_head(peer->fd, peer->text, sizeof(peer->text));
    if (peer->ok) {
        const char response[] = "HTTP/1.1 200 Connection Established\r\n\r\n";
        peer->ok = send(peer->fd, response, sizeof(response) - 1, 0) == (ssize_t)sizeof(response) - 1;
    }
    return NULL;
}

static int basic_headers(void) {
    struct target target = {0};
    target.host = "origin.test"; target.port = "443";
    target.authority = "origin.test"; target.authority_length = strlen(target.authority);
    target.path = "/resource"; target.proxy_routes = authenticated_route();
    CHECK(next_system_proxy(&target));
    CHECK(target.proxy_authorization && !strcmp(target.proxy_authorization, "Basic dXNlcjpwYXNz"));
    struct exchange exchange = { .url = "http://origin.test/resource", .method = "GET" };
    for (int secure = 0; secure <= 1; secure++) {
        int pair[2]; CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, pair) == 0);
        socket_limit(pair[0]); socket_limit(pair[1]);
        struct channel channel = { .socket = pair[0] };
        target.secure = secure;
        if (secure) exchange.url = "https://origin.test/resource";
        CHECK(send_request(&channel, &exchange, &target));
        char request[2048]; CHECK(fixture_read_head(pair[1], request, sizeof(request)));
        CHECK(strstr(request, secure ? "GET /resource HTTP/1.1" : "GET http://origin.test/resource HTTP/1.1"));
        CHECK(secure ? !strstr(request, "Proxy-Authorization:") :
            !!strstr(request, "Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
        close(pair[0]); close(pair[1]);
    }
    int pair[2]; CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, pair) == 0);
    socket_limit(pair[0]); socket_limit(pair[1]);
    struct channel channel = { .socket = pair[0] };
    struct head_peer peer = { .fd = pair[1] };
    pthread_t thread; CHECK(pthread_create(&thread, NULL, tunnel_peer, &peer) == 0);
    CHECK(open_tunnel(&channel, &target));
    CHECK(pthread_join(thread, NULL) == 0 && peer.ok);
    CHECK(strstr(peer.text, "CONNECT origin.test:443 HTTP/1.1\r\n"));
    CHECK(strstr(peer.text, "Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
    close(pair[0]); close(pair[1]);
    free_target(&target);
    return 1;
}

struct socks_peer { int fd, ok, auth, family; const char *host; unsigned short port; };
static void *socks_server(void *opaque) {
    struct socks_peer *peer = opaque;
    unsigned char bytes[260], address[16];
    peer->ok = read_fully(peer->fd, bytes, 3) && bytes[0] == 5 && bytes[1] == 1 &&
        bytes[2] == (peer->auth ? 2 : 0);
    if (!peer->ok) return NULL;
    unsigned char greeting[] = {5, peer->auth ? 2 : 0};
    if (send(peer->fd, greeting, 2, 0) != 2) { peer->ok = 0; return NULL; }
    if (peer->auth) {
        peer->ok = read_fully(peer->fd, bytes, 2) && bytes[0] == 1 && bytes[1] == 4 &&
            read_fully(peer->fd, bytes, 4) && !memcmp(bytes, "user", 4) &&
            read_fully(peer->fd, bytes, 1) && bytes[0] == 4 &&
            read_fully(peer->fd, bytes, 4) && !memcmp(bytes, "pass", 4);
        unsigned char accepted[] = {1, 0};
        if (!peer->ok || send(peer->fd, accepted, 2, 0) != 2) { peer->ok = 0; return NULL; }
    }
    peer->ok = read_fully(peer->fd, bytes, 4) && bytes[0] == 5 && bytes[1] == 1 &&
        bytes[2] == 0 && bytes[3] == peer->family;
    if (!peer->ok) return NULL;
    size_t length = peer->family == 1 ? 4 : peer->family == 4 ? 16 : strlen(peer->host);
    if (peer->family == 3) peer->ok = read_fully(peer->fd, bytes, 1) && (size_t)bytes[0] == length;
    peer->ok = peer->ok && read_fully(peer->fd, bytes, length + 2);
    if (peer->family == 3) peer->ok = peer->ok && !memcmp(bytes, peer->host, length);
    else peer->ok = peer->ok && inet_pton(peer->family == 1 ? AF_INET : AF_INET6, peer->host, address) == 1 &&
        !memcmp(bytes, address, length);
    peer->ok = peer->ok && bytes[length] == (peer->port >> 8) && bytes[length + 1] == (peer->port & 255);
    unsigned char response[] = {5, 0, 0, 1, 127, 0, 0, 1, 0, 0};
    if (peer->ok) peer->ok = send(peer->fd, response, sizeof(response), 0) == (ssize_t)sizeof(response);
    return NULL;
}

static int socks_wire(void) {
    const struct { const char *host; int family, auth; } cases[] = {
        {"192.0.2.1", 1, 0}, {"2001:db8::1", 4, 0}, {"origin.test", 3, 1}
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        int pair[2]; CHECK(socketpair(AF_UNIX, SOCK_STREAM, 0, pair) == 0);
        socket_limit(pair[0]); socket_limit(pair[1]);
        struct channel channel = { .socket = pair[0] };
        struct target target = { .host = (char *)cases[i].host, .port = "8443" };
        if (cases[i].auth) { target.proxy_user = "user"; target.proxy_password = "pass"; }
        struct socks_peer peer = { .fd = pair[1], .auth = cases[i].auth, .family = cases[i].family,
            .host = cases[i].host, .port = 8443 };
        pthread_t thread; CHECK(pthread_create(&thread, NULL, socks_server, &peer) == 0);
        int connected = open_socks(&channel, &target);
        CHECK(pthread_join(thread, NULL) == 0 && connected && peer.ok);
        close(pair[0]); close(pair[1]);
    }
    return 1;
}

int main(void) {
    alarm(30);
    @autoreleasepool {
        if (!pac_order() || !fallback_route() || !basic_headers() || !socks_wire()) return 1;
    }
    puts("http-proxy-fixture: passed");
    return 0;
}
