// macOS HTTP adapter using CFNetwork's blocking read stream for prompt body delivery.
// CFReadStreamRead returns available bytes without waiting for the full response.
#import <Foundation/Foundation.h>
#import <CFNetwork/CFNetwork.h>
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

// CFNetwork's HTTP stream is deprecated but is the system API with a documented
// partial-read contract; NSURLSession can coalesce a short unfinished response.
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

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
    if (fprintf(stdout, "HTTP/1.1 %ld Response\r\n", (long)status) < 0) {
        CFRelease(fields);
        return 0;
    }
    for (NSString *key in headers) {
        NSString *line = [NSString stringWithFormat:@"%@: %@\r\n", key, headers[key]];
        NSData *bytes = [line dataUsingEncoding:NSISOLatin1StringEncoding allowLossyConversion:YES];
        if (!bytes || fwrite(bytes.bytes, 1, bytes.length, stdout) != bytes.length) {
            CFRelease(fields);
            return 0;
        }
    }
    CFRelease(fields);
    return fwrite("\r\n", 1, 2, stdout) == 2 && fflush(stdout) == 0;
}

int main(void) {
    @autoreleasepool {
        int result = 1;
        char *line = NULL;
        size_t capacity = 0;
        size_t size = 0;
        NSString *url_text = nil, *method = nil, *host = nil;
        NSURL *url = nil;
        NSData *body = nil;
        CFHTTPMessageRef request = NULL, response = NULL;
        CFReadStreamRef stream = NULL;
        CFDictionaryRef proxy = NULL;
        unsigned long long timeout_ms = 0, count = 0;
        unsigned long long expected = 0, received = 0;
        int has_expected = 0, head = 0;
        BOOL loopback = NO;
        CFIndex first = 0;
        unsigned char buffer[16384];

        if (getline(&line, &capacity, stdin) < 0) goto done;
        url_text = decoded(line);
        if (getline(&line, &capacity, stdin) < 0) goto done;
        method = decoded(line);
        if (getline(&line, &capacity, stdin) < 0 || !parse_decimal(line, &timeout_ms) ||
            !timeout_ms || timeout_ms > (unsigned long long)UINT_MAX * 1000) goto done;
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
                [value containsString:@"\r"] || [value containsString:@"\n"]) goto done;
            CFHTTPMessageSetHeaderFieldValue(request, (__bridge CFStringRef)name, (__bridge CFStringRef)value);
        }
        if (getline(&line, &capacity, stdin) < 0) goto done;
        size = strlen(line);
        if (size && line[size - 1] == '\n') line[--size] = 0;
        if (size && line[size - 1] == '\r') line[size - 1] = 0;
        body = [[NSData alloc] initWithBase64EncodedString:[NSString stringWithUTF8String:line] options:0];
        if (!body) goto done;
        if (body.length || [method isEqual:@"POST"] || [method isEqual:@"PUT"] || [method isEqual:@"PATCH"])
            CFHTTPMessageSetBody(request, (__bridge CFDataRef)body);
        head = [method isEqual:@"HEAD"];

        stream = CFReadStreamCreateForHTTPRequest(NULL, request);
        if (!stream) goto done;
        host = url.host.lowercaseString;
        loopback = [host isEqual:@"localhost"] || [host isEqual:@"::1"] || [host hasPrefix:@"127."];
        if (!loopback) {
            proxy = CFNetworkCopySystemProxySettings();
            if (proxy && !CFReadStreamSetProperty(stream, kCFStreamPropertyHTTPProxy, proxy)) goto done;
        }
        alarm((unsigned)((timeout_ms + 999) / 1000));
        if (!CFReadStreamOpen(stream)) goto done;
        first = CFReadStreamRead(stream, buffer, sizeof(buffer));
        response = (CFHTTPMessageRef)CFReadStreamCopyProperty(stream, kCFStreamPropertyHTTPResponseHeader);
        if (!response || !response_headers(response, head, &expected, &has_expected)) goto done;
        if (first < 0) goto done;
        if (!head) {
            CFIndex length = first;
            for (;;) {
                if (length == 0) break;
                received += (unsigned long long)length;
                if (has_expected && received > expected) goto done;
                if (fwrite(buffer, 1, (size_t)length, stdout) != (size_t)length || fflush(stdout)) goto done;
                length = CFReadStreamRead(stream, buffer, sizeof(buffer));
                if (length < 0) goto done;
            }
            if (has_expected && received != expected) goto done;
        }
        result = 0;
done:
        alarm(0);
        if (stream) { CFReadStreamClose(stream); CFRelease(stream); }
        if (response) CFRelease(response);
        if (request) CFRelease(request);
        if (proxy) CFRelease(proxy);
        free(line);
        if (result) fputs("Native HTTP request failed\n", stderr);
        return result;
    }
}
