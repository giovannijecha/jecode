// Resolve the OS proxy policy, including PAC. Fall back only through the routes
// explicitly returned by the OS, preserving their order.
struct proxy_result { CFArrayRef proxies; int done; };

static void proxy_ready(void *context, CFArrayRef proxies, CFErrorRef error) {
    struct proxy_result *result = context;
    (void)error;
    if (proxies) result->proxies = CFRetain(proxies);
    result->done = 1;
    CFRunLoopStop(CFRunLoopGetCurrent());
}

static CFArrayRef proxy_script(CFDictionaryRef proxy, CFStringRef type, CFURLRef url) {
    if (CFEqual(type, kCFProxyTypeAutoConfigurationJavaScript)) {
        CFStringRef script = CFDictionaryGetValue(proxy, kCFProxyAutoConfigurationJavaScriptKey);
        return script ? CFNetworkCopyProxiesForAutoConfigurationScript(script, url, NULL) : NULL;
    }
    CFURLRef location = CFDictionaryGetValue(proxy, kCFProxyAutoConfigurationURLKey);
    if (!location) return NULL;
    struct proxy_result result = {0};
    CFStreamClientContext context = {0, &result, NULL, NULL, NULL};
    CFRunLoopSourceRef source = CFNetworkExecuteProxyAutoConfigurationURL(location, url, proxy_ready, &context);
    if (!source) return NULL;
    CFRunLoopRef loop = CFRunLoopGetCurrent();
    CFRunLoopAddSource(loop, source, kCFRunLoopDefaultMode);
    CFAbsoluteTime deadline = CFAbsoluteTimeGetCurrent() + 10;
    while (!result.done && CFAbsoluteTimeGetCurrent() < deadline)
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, deadline - CFAbsoluteTimeGetCurrent(), false);
    CFRunLoopSourceInvalidate(source);
    CFRunLoopRemoveSource(loop, source, kCFRunLoopDefaultMode);
    CFRelease(source);
    return result.proxies;
}

static char *proxy_string(CFDictionaryRef proxy, CFStringRef key) {
    CFStringRef value = CFDictionaryGetValue(proxy, key);
    if (!value || CFGetTypeID(value) != CFStringGetTypeID()) return NULL;
    CFIndex capacity = CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), kCFStringEncodingUTF8) + 1;
    char *text = malloc((size_t)capacity);
    if (text && !CFStringGetCString(value, text, capacity, kCFStringEncodingUTF8)) { free(text); return NULL; }
    return text;
}

static void proxy_routes(CFMutableArrayRef routes, CFArrayRef proxies, CFURLRef url, int depth) {
    for (CFIndex index = 0; proxies && index < CFArrayGetCount(proxies); index++) {
        CFDictionaryRef proxy = CFArrayGetValueAtIndex(proxies, index);
        CFStringRef type = CFDictionaryGetValue(proxy, kCFProxyTypeKey);
        if (type && (CFEqual(type, kCFProxyTypeAutoConfigurationURL) ||
                     CFEqual(type, kCFProxyTypeAutoConfigurationJavaScript))) {
            CFArrayRef expanded = depth < 4 ? proxy_script(proxy, type, url) : NULL;
            if (expanded) { proxy_routes(routes, expanded, url, depth + 1); CFRelease(expanded); }
        } else {
            CFArrayAppendValue(routes, proxy);
        }
    }
}

static int next_system_proxy(struct target *target) {
    if (!target->proxy_routes) return 0;
    free(target->proxy_hostport); target->proxy_hostport = NULL;
    free(target->proxy_user); target->proxy_user = NULL;
    free(target->proxy_password); target->proxy_password = NULL;
    free(target->proxy_authorization); target->proxy_authorization = NULL;
    target->connect_host = target->host;
    target->connect_port = target->port;
    target->proxy = target->socks = 0;
    while (target->proxy_index < CFArrayGetCount(target->proxy_routes)) {
        CFDictionaryRef proxy = CFArrayGetValueAtIndex(target->proxy_routes, target->proxy_index++);
        CFStringRef type = CFDictionaryGetValue(proxy, kCFProxyTypeKey);
        if (type && CFEqual(type, kCFProxyTypeNone)) return 1;
        int socks = type && CFEqual(type, kCFProxyTypeSOCKS);
        if (!type || (!socks && !CFEqual(type, kCFProxyTypeHTTP) && !CFEqual(type, kCFProxyTypeHTTPS))) {
            fail('E', "Native HTTP proxy type is unsupported");
            continue;
        }
        char *host = proxy_string(proxy, kCFProxyHostNameKey);
        CFNumberRef port = CFDictionaryGetValue(proxy, kCFProxyPortNumberKey);
        int number = 0;
        if (!host || !port || !CFNumberGetValue(port, kCFNumberIntType, &number) || number < 1 || number > 65535) {
            free(host);
            fail('E', "Native HTTP proxy address is invalid");
            continue;
        }
        size_t length = strlen(host);
        target->proxy_hostport = malloc(length + 8);
        if (!target->proxy_hostport) { free(host); return fail('E', "Native HTTP is out of memory"); }
        memcpy(target->proxy_hostport, host, length + 1);
        target->connect_host = target->proxy_hostport;
        target->connect_port = target->proxy_hostport + length + 1;
        snprintf(target->connect_port, 7, "%d", number);
        free(host);
        target->socks = socks;
        target->proxy = !socks;
        target->proxy_user = proxy_string(proxy, kCFProxyUsernameKey);
        target->proxy_password = proxy_string(proxy, kCFProxyPasswordKey);
        if (!socks && target->proxy_user && target->proxy_password) {
            size_t size = strlen(target->proxy_user) + strlen(target->proxy_password) + 1;
            char *plain = malloc(size + 1);
            target->proxy_authorization = malloc((size + 2) / 3 * 4 + 7);
            if (!plain || !target->proxy_authorization) { free(plain); return fail('E', "Native HTTP is out of memory"); }
            snprintf(plain, size + 1, "%s:%s", target->proxy_user, target->proxy_password);
            memcpy(target->proxy_authorization, "Basic ", 6);
            base64_encode((const unsigned char *)plain, size, target->proxy_authorization + 6);
            free(plain);
        }
        return 1;
    }
    return 0;
}

static int system_proxy(struct target *target, const char *text) {
    CFStringRef string = CFStringCreateWithCString(NULL, text, kCFStringEncodingUTF8);
    CFURLRef url = string ? CFURLCreateWithString(NULL, string, NULL) : NULL;
    CFDictionaryRef settings = CFNetworkCopySystemProxySettings();
    CFArrayRef proxies = url && settings ? CFNetworkCopyProxiesForURL(url, settings) : NULL;
    CFMutableArrayRef routes = CFArrayCreateMutable(NULL, 0, &kCFTypeArrayCallBacks);
    if (string) CFRelease(string);
    if (settings) CFRelease(settings);
    if (routes) proxy_routes(routes, proxies, url, 0);
    if (proxies) CFRelease(proxies);
    if (url) CFRelease(url);
    target->proxy_routes = routes;
    // A PAC file out of reach, as on a laptop away from its network, leaves no route.
    // Connect directly, as browsers do, rather than refusing every request.
    if (routes && CFArrayGetCount(routes) == 0) return 1;
    return next_system_proxy(target) ? 1 : fail('E', "Native HTTP proxy configuration failed");
}

static int open_socks(struct channel *channel, const struct target *target) {
    unsigned char reply[4], wire[520];
    int authenticate = target->proxy_user && target->proxy_password;
    unsigned char greeting[] = {5, 1, authenticate ? 2 : 0};
    if (!send_bytes(channel, greeting, sizeof(greeting)) || !receive_exact(channel, reply, 2))
        return fail('T', "Native HTTP SOCKS negotiation failed");
    if (reply[0] != 5 || reply[1] != greeting[2]) return fail('E', "Native HTTP SOCKS authentication was refused");
    if (authenticate) {
        size_t user = strlen(target->proxy_user), password = strlen(target->proxy_password);
        if (user > 255 || password > 255) return fail('E', "Native HTTP SOCKS credentials are too long");
        wire[0] = 1; wire[1] = (unsigned char)user;
        memcpy(wire + 2, target->proxy_user, user);
        wire[2 + user] = (unsigned char)password;
        memcpy(wire + 3 + user, target->proxy_password, password);
        if (!send_bytes(channel, wire, 3 + user + password) || !receive_exact(channel, reply, 2))
            return fail('T', "Native HTTP SOCKS authentication failed");
        if (reply[0] != 1 || reply[1] != 0) return fail('E', "Native HTTP SOCKS authentication was refused");
    }
    size_t host = strlen(target->host);
    unsigned long port = strtoul(target->port, NULL, 10);
    if (host > 255 || !port || port > 65535) return fail('E', "Native HTTP SOCKS target is invalid");
    wire[0] = 5; wire[1] = 1; wire[2] = 0;
    size_t size = 4;
    if (inet_pton(AF_INET, target->host, wire + 4) == 1) { wire[3] = 1; size += 4; }
    else if (inet_pton(AF_INET6, target->host, wire + 4) == 1) { wire[3] = 4; size += 16; }
    else { wire[3] = 3; wire[size++] = (unsigned char)host; memcpy(wire + size, target->host, host); size += host; }
    wire[size++] = (unsigned char)(port >> 8); wire[size++] = (unsigned char)port;
    if (!send_bytes(channel, wire, size) || !receive_exact(channel, reply, 4))
        return fail('T', "Native HTTP SOCKS connection failed");
    if (reply[0] != 5 || reply[1] != 0) return fail('E', "Native HTTP SOCKS connection was refused");
    size_t address = reply[3] == 1 ? 4 : reply[3] == 4 ? 16 : 0;
    if (reply[3] == 3) { if (!receive_exact(channel, wire, 1)) return 0; address = wire[0]; }
    if (!address || !receive_exact(channel, wire, address + 2)) return fail('T', "Native HTTP SOCKS response is invalid");
    return 1;
}
