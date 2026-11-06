#define main jecode_http_entry
#include "../native/linux_http.c"
#undef main

static int ticket_socket, ticket_peer, ticket_mode, ticket_seen, ticket_replied;
static const unsigned char answer[] = {0x81, 2, 'o', 'k'};

static int ticket_pending(const SSL *ssl) { (void)ssl; return 0; }
static int ticket_peek(SSL *ssl, void *bytes, int count) {
    (void)ssl; (void)count;
    char ticket;
    if (ticket_mode == 2 && ticket_seen && !ticket_replied) {
        if (send(ticket_peer, answer, sizeof(answer), 0) != (ssize_t)sizeof(answer)) return 0;
        ticket_replied = 1;
    }
    if (recv(ticket_socket, &ticket, 1, MSG_PEEK) != 1) return 0;
    if (ticket == 'T') {
        if (recv(ticket_socket, &ticket, 1, 0) != 1) return 0;
        ticket_seen = 1;
        if (ticket_mode == 1 && write(STDOUT_FILENO, "T", 1) != 1) return 0;
        return -1;
    }
    *(unsigned char *)bytes = (unsigned char)ticket;
    return 1;
}
static int ticket_read(SSL *ssl, void *bytes, int count) {
    (void)ssl;
    char ticket;
    if (recv(ticket_socket, &ticket, 1, MSG_PEEK) == 1 && ticket == 'T') {
        if (recv(ticket_socket, &ticket, 1, 0) != 1) return -1;
        if (ticket_mode == 1 && write(STDOUT_FILENO, "T", 1) != 1) return -1;
        struct pollfd later = { .fd = ticket_socket, .events = POLLIN };
        (void)poll(&later, 1, 300); // Models a blocking SSL_read waiting for application data.
        return -1;
    }
    return (int)recv(ticket_socket, bytes, (size_t)count, 0);
}
static int ticket_write(SSL *ssl, const void *bytes, int count) {
    (void)ssl; (void)bytes;
    if (!ticket_replied && send(ticket_peer, answer, sizeof(answer), 0) == (ssize_t)sizeof(answer)) {
        ticket_replied = 1;
        return count;
    }
    return -1;
}
static int ticket_error(const SSL *ssl, int count) {
    (void)ssl; (void)count;
    return ticket_mode == 2 && !ticket_replied ? 3 : 2; // WANT_WRITE or WANT_READ
}
static void ticket_clear(void) {}

static int late_ticket(int mode) {
    int pair[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, pair)) return 11;
    ticket_socket = pair[0];
    ticket_peer = pair[1];
    ticket_mode = mode;
    if (send(pair[1], "T", 1, 0) != 1) return 12;
    ssl_pending = ticket_pending;
    ssl_peek = ticket_peek;
    ssl_read = ticket_read;
    ssl_write = ticket_write;
    ssl_get_error = ticket_error;
    err_clear_error = ticket_clear;
    struct channel channel = { .socket = pair[0], .ssl = (void *)1 };
    operation_limit_ms = 500;
    alarm(2);
    socket_loop(&channel, 250);
    return 10; // A TLS control record must not end the socket before its idle deadline.
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "late-ticket-input")) return late_ticket(1);
    if (argc == 2 && !strcmp(argv[1], "late-ticket-write")) return late_ticket(2);
    if (setenv("HTTP_PROXY", "http://proxy.fixture.test:18080", 1) ||
        setenv("NO_PROXY", "", 1)) return 1;
    struct target external = {0};
    if (!parse_target("http://127.example.test/", &external)) return 2;
    int proxied = external.proxy && !strcmp(external.connect_host, "proxy.fixture.test") &&
        !strcmp(external.connect_port, "18080");
    free_target(&external);
    if (!proxied) return 3;
    struct target local = {0};
    if (!parse_target("http://127.0.0.2/", &local)) return 4;
    int direct = !local.proxy && !strcmp(local.connect_host, "127.0.0.2");
    free_target(&local);
    return direct ? 0 : 5;
}
