// TLS over an already connected OS socket. CFStream owns TLS verification;
// the channel owns the descriptor and the HTTP exchange above it.
struct mac_tls { CFReadStreamRef read; CFWriteStreamRef write; };

static void tls_free(struct channel *channel) {
    struct mac_tls *tls = channel->ssl;
    if (tls->read) { CFReadStreamClose(tls->read); CFRelease(tls->read); }
    if (tls->write) { CFWriteStreamClose(tls->write); CFRelease(tls->write); }
    free(tls);
}

static int open_tls(struct channel *channel, const char *host) {
    struct mac_tls *tls = calloc(1, sizeof(*tls));
    if (!tls) return fail('E', "Native HTTP is out of memory");
    channel->ssl = tls;
    CFStreamCreatePairWithSocket(NULL, channel->socket, &tls->read, &tls->write);
    if (!tls->read || !tls->write) return fail('E', "Native HTTP could not initialize TLS");
    CFStringRef peer = CFStringCreateWithCString(NULL, host, kCFStringEncodingUTF8);
    if (!peer) return fail('E', "Native HTTP TLS hostname is invalid");
    const void *keys[] = { kCFStreamSSLPeerName, kCFStreamSSLValidatesCertificateChain, kCFStreamSSLLevel };
    const void *values[] = { peer, kCFBooleanTrue, kCFStreamSocketSecurityLevelNegotiatedSSL };
    CFDictionaryRef settings = CFDictionaryCreate(NULL, keys, values, 3,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    int configured = settings &&
        CFReadStreamSetProperty(tls->read, kCFStreamPropertyShouldCloseNativeSocket, kCFBooleanFalse) &&
        CFWriteStreamSetProperty(tls->write, kCFStreamPropertyShouldCloseNativeSocket, kCFBooleanFalse) &&
        CFReadStreamSetProperty(tls->read, kCFStreamPropertySSLSettings, settings) &&
        CFWriteStreamSetProperty(tls->write, kCFStreamPropertySSLSettings, settings);
    if (settings) CFRelease(settings);
    CFRelease(peer);
    if (!configured || !CFReadStreamOpen(tls->read) || !CFWriteStreamOpen(tls->write))
        return fail('E', "Native HTTP could not initialize TLS");
    return 1;
}

static int tls_error(CFStreamError error) {
    return fail(error.domain == kCFStreamErrorDomainSSL ? 'E' : 'T',
        "Native HTTP TLS transfer failed");
}

static int tls_write(struct channel *channel, const void *bytes, int count) {
    struct mac_tls *tls = channel->ssl;
    CFIndex written = CFWriteStreamWrite(tls->write, bytes, count);
    if (written <= 0) tls_error(CFWriteStreamGetError(tls->write));
    return (int)written;
}

static int tls_read(struct channel *channel, void *bytes, int count) {
    struct mac_tls *tls = channel->ssl;
    CFIndex received = CFReadStreamRead(tls->read, bytes, count);
    if (received < 0) tls_error(CFReadStreamGetError(tls->read));
    return (int)received;
}

static int tls_pending(struct channel *channel) {
    struct mac_tls *tls = channel->ssl;
    return CFReadStreamHasBytesAvailable(tls->read);
}

// Readable TLS control traffic cannot be consumed here without risking a
// blocking CFStream read. Replace that idle connection instead.
static int tls_idle_usable(struct channel *channel) { (void)channel; return 0; }
