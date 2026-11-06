// TLS on Linux uses only the OS-supplied OpenSSL ABI.
#include <dlfcn.h>
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
static int (*ssl_peek)(SSL *, void *, int);
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
    SYMBOL(ssl_peek, "SSL_peek");
    SYMBOL(ssl_pending, "SSL_pending");
    /* In libcrypto, which libssl loads; only the WebSocket handshake needs it. */
    *(void **)(&sha1_digest) = dlsym(library, "SHA1");
#undef SYMBOL
    loaded = 1;
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

static void tls_free(struct channel *channel) { ssl_free(channel->ssl); }
static int tls_write(struct channel *channel, const void *bytes, int count) {
    int written = ssl_write(channel->ssl, bytes, count);
    if (written <= 0) fail('T', "Native HTTP send failed");
    return written;
}
static int tls_read(struct channel *channel, void *bytes, int count) { return ssl_read(channel->ssl, bytes, count); }
static int tls_pending(struct channel *channel) { return ssl_pending(channel->ssl); }
static int tls_idle_usable(struct channel *channel) {
    int flags = fcntl(channel->socket, F_GETFL, 0);
    if (flags < 0 || fcntl(channel->socket, F_SETFL, flags | O_NONBLOCK) < 0) return 0;
    unsigned char ignored[256];
    err_clear_error();
    int count = ssl_read(channel->ssl, ignored, sizeof(ignored));
    int blocked = count <= 0 && ssl_get_error(channel->ssl, count) == 2;
    return fcntl(channel->socket, F_SETFL, flags) == 0 && blocked;
}
