// macOS HTTP adapter. It serves requests in order and exits when stdin closes or
// after an idle period; its protocol is winhttp.cs's. HTTP uses CFNetwork's
// blocking read stream for prompt body delivery: CFReadStreamRead returns available
// bytes without waiting for the full response. With the arguments `socket` and an
// idle limit in milliseconds it instead serves one WebSocket connection.
#import <Foundation/Foundation.h>
#import <CFNetwork/CFNetwork.h>
#include <errno.h>
#include <limits.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/event.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

// CFNetwork's HTTP stream is deprecated but is the system API with a documented
// partial-read contract; NSURLSession can coalesce a short unfinished response.
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

enum { IDLE_LIMIT_SECONDS = 300, SOCKET_MESSAGE_LIMIT = 64 * 1024 * 1024 };

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

static void frame_text(char tag, const char *text) {
    frame(tag, text, strlen(text));
}

// Ends the helper when the parent exits, even during a long blocking read. kqueue
// documents EV_EOF for the last writer of a pipe, whether or not data is pending;
// EV_CLEAR keeps unread request bytes from waking this thread repeatedly.
static void *watch_parent(void *unused) {
    (void)unused;
    int queue = kqueue();
    if (queue < 0) return NULL;
    struct kevent change;
    EV_SET(&change, STDIN_FILENO, EVFILT_READ, EV_ADD | EV_CLEAR, 0, 0, NULL);
    if (kevent(queue, &change, 1, NULL, 0, NULL) < 0) return NULL;
    for (;;) {
        struct kevent event;
        int ready = kevent(queue, NULL, 0, &event, 1, NULL);
        if (ready > 0 && (event.flags & EV_EOF)) _exit(0);
        if (ready < 0 && errno != EINTR) return NULL;
    }
}

static void start_parent_watch(void) {
    pthread_t thread;
    if (pthread_create(&thread, NULL, watch_parent, NULL) == 0) pthread_detach(thread);
}

static NSString *decoded(char *line) {
    if (!line) return nil;
    size_t size = strlen(line);
    if (size && line[size - 1] == '\n') line[--size] = 0;
    if (size && line[size - 1] == '\r') line[size - 1] = 0;
    NSData *data = [[NSData alloc] initWithBase64EncodedString:[NSString stringWithUTF8String:line] options:0];
    return data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;
}

static int parse_decimal(const char *text, unsigned long long *value) {
    if (!text || *text < '0' || *text > '9') return 0;
    errno = 0;
    char *end;
    *value = strtoull(text, &end, 10);
    return !errno && (end[0] == '\n' || end[0] == '\r' || end[0] == 0);
}

static int transport_error(CFReadStreamRef stream, int before_headers) {
    CFErrorRef detail = CFReadStreamCopyError(stream);
    if (detail) {
        CFStringRef domain = CFErrorGetDomain(detail);
        CFIndex code = CFErrorGetCode(detail);
        int network = CFEqual(domain, kCFErrorDomainCFNetwork) ||
                      CFEqual(domain, (__bridge CFStringRef)NSURLErrorDomain);
        int retry = network && (code == kCFErrorHTTPConnectionLost ||
            code == kCFURLErrorTimedOut || code == kCFURLErrorCannotFindHost ||
            code == kCFURLErrorCannotConnectToHost ||
            code == kCFURLErrorNetworkConnectionLost ||
            code == kCFURLErrorDNSLookupFailed ||
            code == kCFURLErrorNotConnectedToInternet ||
            (before_headers && code == kCFErrorHTTPParseFailure));
        CFRelease(detail);
        if (retry) return 1;
    }
    CFStreamError error = CFReadStreamGetError(stream);
    return error.domain == kCFStreamErrorDomainPOSIX ||
           error.domain == kCFStreamErrorDomainNetDB ||
           (before_headers && error.domain == kCFStreamErrorDomainHTTP &&
            error.error == kCFStreamErrorHTTPParseFailure);
}

static int response_headers(CFHTTPMessageRef response, int head,
                            unsigned long long *expected, int *has_expected) {
    CFIndex status = CFHTTPMessageGetResponseStatusCode(response);
    if (status < 100 || status > 999) return 0;
    CFDictionaryRef fields = CFHTTPMessageCopyAllHeaderFields(response);
    if (!fields) return 0;
    NSDictionary *headers = (__bridge NSDictionary *)fields;
    NSString *length = CFBridgingRelease(CFHTTPMessageCopyHeaderFieldValue(response, CFSTR("Content-Length")));
    NSString *encoding = CFBridgingRelease(CFHTTPMessageCopyHeaderFieldValue(response, CFSTR("Transfer-Encoding")));
    *has_expected = length != nil && !head && status >= 200 && status != 204 && status != 304;
    if (length) {
        const char *digits = length.UTF8String;
        if (!digits || !*digits || *digits < '0' || *digits > '9') { CFRelease(fields); return 0; }
        errno = 0;
        char *end;
        *expected = strtoull(digits, &end, 10);
        if (errno || *end) { CFRelease(fields); return 0; }
    }
    if (length && encoding) { CFRelease(fields); return 0; }
    NSMutableData *text = [NSMutableData data];
    [text appendData:[[NSString stringWithFormat:@"HTTP/1.1 %ld Response\r\n", (long)status]
        dataUsingEncoding:NSISOLatin1StringEncoding]];
    for (NSString *key in headers) {
        NSString *line = [NSString stringWithFormat:@"%@: %@\r\n", key, headers[key]];
        NSData *bytes = [line dataUsingEncoding:NSISOLatin1StringEncoding allowLossyConversion:YES];
        if (!bytes) { CFRelease(fields); return 0; }
        [text appendData:bytes];
    }
    CFRelease(fields);
    [text appendBytes:"\r\n" length:2];
    frame('D', text.bytes, text.length);
    return 1;
}

// One request, reported in frames. Returns 0 at the end of stdin; < 0 when invalid.
static int serve_request(void) {
    int result = 1;
    char *line = NULL;
    size_t capacity = 0;
    size_t size = 0;
    NSString *url_text = nil, *method = nil, *host = nil;
    NSURL *url = nil;
    NSMutableData *body = nil;
    CFHTTPMessageRef request = NULL, response = NULL;
    CFReadStreamRef stream = NULL;
    CFStreamError stream_error = { 0, 0 };
    CFDictionaryRef proxy = NULL;
    unsigned long long timeout_ms = 0, count = 0, body_size = 0;
    unsigned long long expected = 0, received = 0;
    int has_expected = 0, head = 0, invalid = 0;
    const char *invalid_reason = "Native HTTP request is invalid";
    BOOL loopback = NO;
    CFIndex first = 0;
    unsigned char buffer[16384];

    // The idle limit covers only the wait for a request.
    alarm(IDLE_LIMIT_SECONDS);
    if (getline(&line, &capacity, stdin) < 0) { alarm(0); free(line); return 0; }
    alarm(0);
    invalid = 1;
    url_text = decoded(line);
    if (getline(&line, &capacity, stdin) < 0) goto done;
    method = decoded(line);
    // A zero timeout means no total limit; the caller enforces inactivity and deadlines.
    if (getline(&line, &capacity, stdin) < 0 || !parse_decimal(line, &timeout_ms) ||
        timeout_ms > (unsigned long long)UINT_MAX * 1000) goto done;
    if (getline(&line, &capacity, stdin) < 0 || !parse_decimal(line, &count) || count > 1000) goto done;
    if (!url_text || !method) goto done;
    url = [NSURL URLWithString:url_text];
    if (!url || !([url.scheme isEqual:@"http"] || [url.scheme isEqual:@"https"])) goto done;
    request = CFHTTPMessageCreateRequest(NULL, (__bridge CFStringRef)method,
        (__bridge CFURLRef)url, kCFHTTPVersion1_1);
    if (!request) goto done;
    for (unsigned long long i = 0; i < count; i++) {
        if (getline(&line, &capacity, stdin) < 0) goto done;
        NSString *name = decoded(line);
        if (getline(&line, &capacity, stdin) < 0) goto done;
        NSString *value = decoded(line);
        if (!name || !value || [name containsString:@"\r"] || [name containsString:@"\n"] ||
            [value containsString:@"\r"] || [value containsString:@"\n"]) {
            invalid_reason = "Native HTTP request header is invalid";
            goto done;
        }
        CFHTTPMessageSetHeaderFieldValue(request, (__bridge CFStringRef)name, (__bridge CFStringRef)value);
    }
    // The raw body follows its decimal length line.
    if (getline(&line, &capacity, stdin) < 0 || !parse_decimal(line, &body_size) ||
        body_size >= SIZE_MAX) goto done;
    size = (size_t)body_size;
    body = [NSMutableData dataWithLength:size];
    if (!body || (size && fread(body.mutableBytes, 1, size, stdin) != size)) goto done;
    invalid = 0;
    frame('A', NULL, 0);
    if (body.length || [method isEqual:@"POST"] || [method isEqual:@"PUT"] || [method isEqual:@"PATCH"])
        CFHTTPMessageSetBody(request, (__bridge CFDataRef)body);
    head = [method isEqual:@"HEAD"];

    stream = CFReadStreamCreateForHTTPRequest(NULL, request);
    if (!stream) goto done;
    // Later requests to the same host then skip connecting when the system can.
    CFReadStreamSetProperty(stream, kCFStreamPropertyHTTPAttemptPersistentConnection, kCFBooleanTrue);
    host = url.host.lowercaseString;
    loopback = [host isEqual:@"localhost"] || [host isEqual:@"::1"] || [host hasPrefix:@"127."];
    if (!loopback) {
        proxy = CFNetworkCopySystemProxySettings();
        if (proxy && !CFReadStreamSetProperty(stream, kCFStreamPropertyHTTPProxy, proxy)) goto done;
    }
    if (!CFReadStreamOpen(stream)) {
        if (transport_error(stream, 1)) result = 2;
        goto done;
    }
    first = CFReadStreamRead(stream, buffer, sizeof(buffer));
    if (first < 0) {
        if (transport_error(stream, 1)) result = 2;
        goto done;
    }
    response = (CFHTTPMessageRef)CFReadStreamCopyProperty(stream, kCFStreamPropertyHTTPResponseHeader);
    if (!response && first == 0) result = 2;
    if (!response || !response_headers(response, head, &expected, &has_expected)) goto done;
    if (!head) {
        CFIndex length = first;
        for (;;) {
            if (length == 0) break;
            received += (unsigned long long)length;
            if (has_expected && received > expected) goto done;
            frame('D', buffer, (size_t)length);
            // A framed response is complete at its length; a later close must not fail it.
            if (has_expected && received == expected) break;
            length = CFReadStreamRead(stream, buffer, sizeof(buffer));
            if (length < 0) {
                if (transport_error(stream, 0)) result = 2;
                goto done;
            }
        }
        if (has_expected && received != expected) { result = 2; goto done; }
    }
    result = 0;
done:
    if (result && stream) stream_error = CFReadStreamGetError(stream);
    if (stream) { CFReadStreamClose(stream); CFRelease(stream); }
    if (response) CFRelease(response);
    if (request) CFRelease(request);
    if (proxy) CFRelease(proxy);
    free(line);
    if (invalid) {
        frame('A', NULL, 0);
        frame_text('E', invalid_reason);
        return -1;
    }
    if (result == 0) {
        frame('E', NULL, 0);
    } else {
        char message[128];
        snprintf(message, sizeof(message), "Native HTTP request failed (stream domain %ld, code %ld)",
                 (long)stream_error.domain, (long)stream_error.error);
        frame_text(result == 2 ? 'T' : 'E', message);
    }
    return 1;
}

@interface SocketDelegate : NSObject <NSURLSessionWebSocketDelegate>
@property(nonatomic) NSURLSessionWebSocketTask *task;
@end

static dispatch_queue_t activity_queue;
static dispatch_source_t idle_timer;
static NSURLSession *socket_session;
static uint64_t last_activity;

static void touch_activity(void) {
    dispatch_sync(activity_queue, ^{ last_activity = clock_gettime_nsec_np(CLOCK_MONOTONIC); });
}

// Runs on the callback queue, the only writer of frames once the connection starts.
// _exit, because exit could wait on the stdin lock held by the blocked main thread.
static void socket_ended(NSString *reason) {
    frame_text('E', reason.UTF8String ?: "Native HTTP socket failed");
    _exit(1);
}

@implementation SocketDelegate
- (void)receiveNext {
    [self.task receiveMessageWithCompletionHandler:^(NSURLSessionWebSocketMessage *message, NSError *error) {
        if (error || !message) {
            socket_ended(@"Native HTTP socket failed");
            return;
        }
        touch_activity();
        NSData *bytes = message.type == NSURLSessionWebSocketMessageTypeString
            ? [message.string dataUsingEncoding:NSUTF8StringEncoding] : message.data;
        frame('D', bytes.bytes, bytes.length);
        [self receiveNext];
    }];
}
- (void)URLSession:(NSURLSession *)session
      webSocketTask:(NSURLSessionWebSocketTask *)webSocketTask
    didOpenWithProtocol:(NSString *)protocol {
    (void)session; (void)webSocketTask; (void)protocol;
    touch_activity();
    frame('A', NULL, 0);
    [self receiveNext];
}
- (void)URLSession:(NSURLSession *)session
      webSocketTask:(NSURLSessionWebSocketTask *)webSocketTask
   didCloseWithCode:(NSURLSessionWebSocketCloseCode)closeCode
             reason:(NSData *)reason {
    (void)session; (void)webSocketTask; (void)closeCode; (void)reason;
    socket_ended(@"Native HTTP socket was closed");
}
- (void)URLSession:(NSURLSession *)session
                    task:(NSURLSessionTask *)task
    didCompleteWithError:(NSError *)error {
    (void)session; (void)task; (void)error;
    socket_ended(@"Native HTTP socket failed");
}
@end

// One WebSocket connection, in winhttp_socket.cs's protocol. Failures end the helper
// with 'E'; the caller then uses HTTP, so nothing here can make a request fail.
static int serve_socket(const char *idle_text) {
    unsigned long long idle_ms = 0;
    if (!parse_decimal(idle_text, &idle_ms) || idle_ms == 0) return 2;
    activity_queue = dispatch_queue_create("jecode.socket.activity", DISPATCH_QUEUE_SERIAL);
    touch_activity();
    char *line = NULL;
    size_t capacity = 0;
    unsigned long long count = 0;
    if (getline(&line, &capacity, stdin) < 0) { free(line); return 1; }
    NSString *url_text = decoded(line);
    if (getline(&line, &capacity, stdin) < 0 || !parse_decimal(line, &count) || count > 1000) {
        frame_text('E', "Native HTTP request is invalid");
        return 1;
    }
    NSURLComponents *components = url_text ? [NSURLComponents componentsWithString:url_text] : nil;
    if (!components) {
        frame_text('E', "Native HTTP URL is invalid");
        return 1;
    }
    components.scheme = [components.scheme isEqual:@"https"] ? @"wss" : @"ws";
    NSMutableURLRequest *request = [NSMutableURLRequest requestWithURL:components.URL];
    for (unsigned long long i = 0; i < count; i++) {
        if (getline(&line, &capacity, stdin) < 0) { free(line); return 1; }
        NSString *name = decoded(line);
        if (getline(&line, &capacity, stdin) < 0) { free(line); return 1; }
        NSString *value = decoded(line);
        if (!name || !value) {
            frame_text('E', "Native HTTP request header is invalid");
            return 1;
        }
        [request setValue:value forHTTPHeaderField:name];
    }
    NSOperationQueue *callbacks = [[NSOperationQueue alloc] init];
    callbacks.maxConcurrentOperationCount = 1;
    SocketDelegate *delegate = [[SocketDelegate alloc] init];
    socket_session = [NSURLSession
        sessionWithConfiguration:[NSURLSessionConfiguration ephemeralSessionConfiguration]
                        delegate:delegate
                   delegateQueue:callbacks];
    NSURLSessionWebSocketTask *task = [socket_session webSocketTaskWithRequest:request];
    task.maximumMessageSize = SOCKET_MESSAGE_LIMIT;
    delegate.task = task;
    [task resume];

    // The parent replaces a connection that ended, so leaving is always safe.
    idle_timer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, activity_queue);
    uint64_t check = idle_ms / 4 + 1;
    dispatch_source_set_timer(idle_timer, dispatch_time(DISPATCH_TIME_NOW, (int64_t)check * NSEC_PER_MSEC),
                              check * NSEC_PER_MSEC, NSEC_PER_MSEC * 100);
    dispatch_source_set_event_handler(idle_timer, ^{
        if (clock_gettime_nsec_np(CLOCK_MONOTONIC) - last_activity >= idle_ms * NSEC_PER_MSEC) _exit(0);
    });
    dispatch_resume(idle_timer);

    for (;;) {
        @autoreleasepool {
            unsigned long long size = 0;
            if (getline(&line, &capacity, stdin) < 0) _exit(0);
            // The parent only sends valid text; closing makes it open a new connection.
            if (!parse_decimal(line, &size) || size > SOCKET_MESSAGE_LIMIT) _exit(1);
            NSMutableData *message = [NSMutableData dataWithLength:(NSUInteger)size];
            if (!message || (size && fread(message.mutableBytes, 1, (size_t)size, stdin) != size)) _exit(0);
            touch_activity();
            NSString *text = [[NSString alloc] initWithData:message encoding:NSUTF8StringEncoding];
            if (!text) _exit(1);
            [task sendMessage:[[NSURLSessionWebSocketMessage alloc] initWithString:text]
                completionHandler:^(NSError *error) {
                    if (error) socket_ended(@"Native HTTP socket failed");
                    touch_activity();
                }];
        }
    }
}

int main(int argc, char **argv) {
    @autoreleasepool {
        // Writes to a closed connection report an error instead of ending the helper.
        signal(SIGPIPE, SIG_IGN);
        start_parent_watch();
        if (argc == 3 && !strcmp(argv[1], "socket")) return serve_socket(argv[2]);
    }
    for (;;) {
        @autoreleasepool {
            int state = serve_request();
            if (state == 0) return 0;
            if (state < 0) return 1;
        }
    }
}
