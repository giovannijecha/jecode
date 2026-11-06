// macOS HTTP adapter. It serves requests in order and exits when stdin closes or
// after an idle period; its protocol is winhttp.cs's. HTTP framing and replay
// policy are shared with Linux over OS sockets and CFStream TLS. With `socket` and an
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
#include <strings.h>
#include <sys/event.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

// CFStream socket TLS is a system boundary; HTTP framing and retries are owned.
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

enum { IDLE_LIMIT_MS = 300000, CONNECT_LIMIT_MS = 10000, RECEIVE_LIMIT_MS = 120000, STALE_RETRIES = 2 };
enum { IDLE_LIMIT_SECONDS = 300, SOCKET_MESSAGE_LIMIT = 64 * 1024 * 1024 };

#include "unix_http_io.h"
#include "unix_http_net.h"
#include "unix_http_exchange.h"

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
        signal(SIGPIPE, SIG_IGN);
        start_parent_watch();
        if (argc == 3 && !strcmp(argv[1], "socket")) return serve_socket(argv[2]);
        return serve_requests();
    }
}
