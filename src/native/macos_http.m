// macOS HTTP adapter using Foundation's system URL loading and trust policy.
// Stdin protocol matches http.cs; headers and body are streamed on stdout.
#import <Foundation/Foundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

@interface Transfer : NSObject <NSURLSessionDataDelegate, NSURLSessionTaskDelegate>
@property (nonatomic, strong) dispatch_semaphore_t done;
@property (nonatomic) BOOL succeeded;
@property (nonatomic) BOOL head;
@end

@implementation Transfer
- (void)URLSession:(NSURLSession *)session task:(NSURLSessionTask *)task
    willPerformHTTPRedirection:(NSHTTPURLResponse *)response
    newRequest:(NSURLRequest *)request
    completionHandler:(void (^)(NSURLRequest *))completionHandler {
    (void)session; (void)task; (void)response; (void)request;
    completionHandler(nil);
}
- (void)URLSession:(NSURLSession *)session dataTask:(NSURLSessionDataTask *)task
    didReceiveResponse:(NSURLResponse *)response
    completionHandler:(void (^)(NSURLSessionResponseDisposition))completionHandler {
    (void)session; (void)task;
    NSHTTPURLResponse *http = (NSHTTPURLResponse *)response;
    fprintf(stdout, "HTTP/1.1 %ld Response\r\n", (long)http.statusCode);
    for (id key in http.allHeaderFields) {
        NSString *line = [NSString stringWithFormat:@"%@: %@\r\n", key, http.allHeaderFields[key]];
        NSData *bytes = [line dataUsingEncoding:NSISOLatin1StringEncoding allowLossyConversion:YES];
        fwrite(bytes.bytes, 1, bytes.length, stdout);
    }
    fwrite("\r\n", 1, 2, stdout);
    fflush(stdout);
    completionHandler(NSURLSessionResponseAllow);
}
- (void)URLSession:(NSURLSession *)session dataTask:(NSURLSessionDataTask *)task
    didReceiveData:(NSData *)data {
    (void)session; (void)task;
    if (!self.head) {
        fwrite(data.bytes, 1, data.length, stdout);
        fflush(stdout);
    }
}
- (void)URLSession:(NSURLSession *)session task:(NSURLSessionTask *)task
    didCompleteWithError:(NSError *)error {
    (void)session; (void)task;
    self.succeeded = error == nil;
    dispatch_semaphore_signal(self.done);
}
@end

static NSString *decoded(char *line) {
    if (!line) return nil;
    size_t size = strlen(line);
    if (size && line[size - 1] == '\n') line[--size] = 0;
    if (size && line[size - 1] == '\r') line[size - 1] = 0;
    NSData *data = [[NSData alloc] initWithBase64EncodedString:[NSString stringWithUTF8String:line] options:0];
    return data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;
}

int main(void) {
    @autoreleasepool {
        int result = 1;
        char *line = NULL;
        size_t capacity = 0;
        NSString *urlText = nil, *method = nil;
        NSURL *url = nil;
        NSMutableURLRequest *request = nil;
        NSData *body = nil;
        NSURLSessionConfiguration *configuration = nil;
        Transfer *transfer = nil;
        NSURLSession *session = nil;
        double timeout = 0;
        long count = 0;
        size_t size = 0;
        if (getline(&line, &capacity, stdin) < 0) goto done;
        urlText = decoded(line);
        if (getline(&line, &capacity, stdin) < 0) goto done;
        method = decoded(line);
        if (getline(&line, &capacity, stdin) < 0) goto done;
        timeout = strtod(line, NULL) / 1000.0;
        if (getline(&line, &capacity, stdin) < 0) goto done;
        count = strtol(line, NULL, 10);
        if (!urlText || !method || timeout <= 0 || count < 0 || count > 1000) goto done;
        url = [NSURL URLWithString:urlText];
        if (!url || !([url.scheme isEqual:@"http"] || [url.scheme isEqual:@"https"])) goto done;
        request = [NSMutableURLRequest requestWithURL:url];
        request.HTTPMethod = method;
        request.timeoutInterval = timeout;
        request.cachePolicy = NSURLRequestReloadIgnoringLocalCacheData;
        for (long i = 0; i < count; i++) {
            if (getline(&line, &capacity, stdin) < 0) goto done;
            NSString *name = decoded(line);
            if (getline(&line, &capacity, stdin) < 0) goto done;
            NSString *value = decoded(line);
            if (!name || !value || [name containsString:@"\r"] || [name containsString:@"\n"] ||
                [value containsString:@"\r"] || [value containsString:@"\n"]) goto done;
            [request setValue:value forHTTPHeaderField:name];
        }
        if (getline(&line, &capacity, stdin) < 0) goto done;
        size = strlen(line);
        if (size && line[size - 1] == '\n') line[--size] = 0;
        if (size && line[size - 1] == '\r') line[size - 1] = 0;
        body = [[NSData alloc] initWithBase64EncodedString:[NSString stringWithUTF8String:line] options:0];
        if (!body) goto done;
        if (body.length || [method isEqual:@"POST"] || [method isEqual:@"PUT"] || [method isEqual:@"PATCH"])
            request.HTTPBody = body;
        configuration = [NSURLSessionConfiguration ephemeralSessionConfiguration];
        configuration.HTTPShouldSetCookies = NO;
        configuration.HTTPCookieStorage = nil;
        configuration.URLCache = nil;
        configuration.timeoutIntervalForRequest = timeout;
        configuration.timeoutIntervalForResource = timeout;
        transfer = [Transfer new];
        transfer.done = dispatch_semaphore_create(0);
        transfer.head = [method isEqual:@"HEAD"];
        session = [NSURLSession sessionWithConfiguration:configuration delegate:transfer delegateQueue:nil];
        [[session dataTaskWithRequest:request] resume];
        dispatch_semaphore_wait(transfer.done, DISPATCH_TIME_FOREVER);
        [session finishTasksAndInvalidate];
        if (transfer.succeeded) result = 0;
done:
        free(line);
        if (result) fputs("Native HTTP request failed\n", stderr);
        return result;
    }
}
