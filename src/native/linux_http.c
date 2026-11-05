// Linux HTTP/1.1 adapter over OS sockets and the OS-supplied OpenSSL TLS ABI. It
// serves requests in order on reused connections and exits when stdin closes or
// after an idle period. Its protocol is winhttp.cs's. No external program or
// development headers.
#define _GNU_SOURCE /* strcasestr */
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>

enum { IDLE_LIMIT_MS = 300000, CONNECT_LIMIT_MS = 10000, RECEIVE_LIMIT_MS = 120000, STALE_RETRIES = 2 };

#include "unix_http_io.h"
#include "unix_http_net.h"
#include "linux_http_socket.h"
#include "unix_http_exchange.h"

int main(int argc, char **argv) {
    signal(SIGPIPE, SIG_IGN);
    if (argc == 3 && !strcmp(argv[1], "socket")) return serve_socket(argv[2]);
    return serve_requests();
}
