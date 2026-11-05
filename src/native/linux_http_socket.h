// One WebSocket connection per helper process (RFC 6455), selected by the arguments
// `socket` and an idle limit in milliseconds, after which the helper exits without
// a frame. Stdin and stdout are winhttp_socket.cs's: base64 URL, header count and
// base64 name/value pairs, then each message's length on a line and its bytes;
// 'A' once connected, 'D' per whole message, 'E' with the reason when it ends.
#include <time.h>

enum { SOCKET_MESSAGE_LIMIT = 64 * 1024 * 1024, SOCKET_WRITE_LIMIT_MS = 30000 };
enum { OP_CONTINUE = 0, OP_TEXT = 1, OP_BINARY = 2, OP_CLOSE = 8, OP_PING = 9, OP_PONG = 10 };

static long long now_ms(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (long long)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static int random_bytes(unsigned char *bytes, size_t size) {
    int source = open("/dev/urandom", O_RDONLY | O_CLOEXEC);
    if (source < 0) return 0;
    size_t filled = 0;
    while (filled < size) {
        ssize_t count = read(source, bytes + filled, size - filled);
        if (count <= 0) { close(source); return 0; }
        filled += (size_t)count;
    }
    close(source);
    return 1;
}

static void base64_encode(const unsigned char *bytes, size_t size, char *text) {
    static const char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t written = 0;
    for (size_t i = 0; i < size; i += 3) {
        unsigned int block = (unsigned int)bytes[i] << 16;
        if (i + 1 < size) block |= (unsigned int)bytes[i + 1] << 8;
        if (i + 2 < size) block |= bytes[i + 2];
        text[written++] = alphabet[block >> 18 & 63];
        text[written++] = alphabet[block >> 12 & 63];
        text[written++] = i + 1 < size ? alphabet[block >> 6 & 63] : '=';
        text[written++] = i + 2 < size ? alphabet[block & 63] : '=';
    }
    text[written] = 0;
}

// Clients mask every frame with a fresh key.
static int socket_send(struct channel *channel, int opcode, const unsigned char *bytes, size_t size) {
    unsigned char *wire = malloc(size + 14);
    if (!wire) return fail('E', "Native HTTP is out of memory");
    size_t used = 0;
    wire[used++] = (unsigned char)(0x80 | opcode);
    if (size < 126) {
        wire[used++] = (unsigned char)(0x80 | size);
    } else if (size <= 0xffff) {
        wire[used++] = 0x80 | 126;
        wire[used++] = (unsigned char)(size >> 8);
        wire[used++] = (unsigned char)size;
    } else {
        wire[used++] = 0x80 | 127;
        for (int shift = 56; shift >= 0; shift -= 8) wire[used++] = (unsigned char)((uint64_t)size >> shift);
    }
    unsigned char *key = wire + used;
    int sent = random_bytes(key, 4);
    used += 4;
    if (sent) {
        for (size_t i = 0; i < size; i++) wire[used + i] = bytes[i] ^ key[i % 4];
        sent = send_bytes(channel, wire, used + size);
    } else {
        fail('E', "Native HTTP could not generate a socket key");
    }
    free(wire);
    return sent;
}

// Sends the upgrade and checks that the server computed the accept value.
static int socket_upgrade(struct channel *channel, const struct target *target, const struct header *headers, long count) {
    if (!load_tls() || !sha1_digest) return fail('E', "Native HTTP sockets need the system OpenSSL library");
    unsigned char nonce[16], digest[20];
    char key[32], expected[32];
    if (!random_bytes(nonce, sizeof(nonce))) return fail('E', "Native HTTP could not generate a socket key");
    base64_encode(nonce, sizeof(nonce), key);
    char material[128];
    int material_size = snprintf(material, sizeof(material), "%s258EAFA5-E914-47DA-95CA-C5AB0DC85B11", key);
    sha1_digest((const unsigned char *)material, (size_t)material_size, digest);
    base64_encode(digest, sizeof(digest), expected);

    size_t capacity = 2048 + strlen(target->path) + target->authority_length;
    for (long i = 0; i < count; i++) capacity += strlen(headers[i].name) + strlen(headers[i].value) + 4;
    char *request = malloc(capacity);
    if (!request) return fail('E', "Native HTTP is out of memory");
    size_t used = (size_t)snprintf(request, capacity,
        "GET %s%s HTTP/1.1\r\nHost: %.*s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
        "Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n",
        *target->path == '?' ? "/" : "", target->path, (int)target->authority_length, target->authority, key);
    for (long i = 0; i < count; i++)
        used += (size_t)snprintf(request + used, capacity - used, "%s: %s\r\n", headers[i].name, headers[i].value);
    used += (size_t)snprintf(request + used, capacity - used, "\r\n");
    int sent = send_bytes(channel, request, used);
    free(request);
    if (!sent) return 0;

    char line[8192];
    int status = 0, accepted = 0;
    if (!receive_line(channel, line, sizeof(line)) || sscanf(line, "HTTP/%*s %d", &status) != 1)
        return fail('E', "Native HTTP socket upgrade failed");
    if (status != 101) return fail('E', "Native HTTP socket was refused");
    for (;;) {
        if (!receive_line(channel, line, sizeof(line))) return fail('E', "Native HTTP socket upgrade failed");
        if (!strcmp(line, "\r\n")) break;
        if (!strncasecmp(line, "Sec-WebSocket-Accept:", 21)) {
            char *value = line + 21;
            while (*value == ' ' || *value == '\t') value++;
            size_t length = strcspn(value, "\r\n");
            while (length && (value[length - 1] == ' ' || value[length - 1] == '\t')) length--;
            accepted = length == strlen(expected) && !strncmp(value, expected, length);
        }
    }
    return accepted ? 1 : fail('E', "Native HTTP socket answered with a wrong accept value");
}

struct socket_message { unsigned char *bytes; size_t size, capacity; };

// Reads one frame. 1 to continue, 0 when the connection ended.
static int socket_receive(struct channel *channel, struct socket_message *message) {
    unsigned char head[2], key[4];
    if (!receive_exact(channel, head, 2)) return fail('E', "Native HTTP socket failed");
    int final = head[0] & 0x80, opcode = head[0] & 0x0f, masked = head[1] & 0x80;
    uint64_t length = head[1] & 0x7f;
    if (head[0] & 0x70) return fail('E', "Native HTTP socket frame is invalid");
    if (length == 126 || length == 127) {
        unsigned char extended[8];
        size_t size = length == 126 ? 2 : 8;
        if (!receive_exact(channel, extended, size)) return fail('E', "Native HTTP socket failed");
        length = 0;
        for (size_t i = 0; i < size; i++) length = length << 8 | extended[i];
    }
    if (masked && !receive_exact(channel, key, 4)) return fail('E', "Native HTTP socket failed");
    if (length > SOCKET_MESSAGE_LIMIT || message->size + length > SOCKET_MESSAGE_LIMIT)
        return fail('E', "Native HTTP socket message is too long");
    int control = opcode >= 8;
    if (control && (length > 125 || !final)) return fail('E', "Native HTTP socket frame is invalid");
    unsigned char control_bytes[125];
    unsigned char *target = control_bytes;
    if (!control) {
        if (message->size + length + 1 > message->capacity) {
            size_t capacity = (size_t)(message->size + length + 1) * 2;
            unsigned char *grown = realloc(message->bytes, capacity);
            if (!grown) return fail('E', "Native HTTP is out of memory");
            message->bytes = grown;
            message->capacity = capacity;
        }
        target = message->bytes + message->size;
    }
    if (length && !receive_exact(channel, target, (size_t)length)) return fail('E', "Native HTTP socket failed");
    if (masked) for (uint64_t i = 0; i < length; i++) target[i] ^= key[i % 4];
    switch (opcode) {
        case OP_PING: return socket_send(channel, OP_PONG, control_bytes, (size_t)length);
        case OP_PONG: return 1;
        case OP_CLOSE: return fail('E', "Native HTTP socket was closed");
        case OP_TEXT: case OP_BINARY: case OP_CONTINUE: break;
        default: return fail('E', "Native HTTP socket frame is invalid");
    }
    message->size += (size_t)length;
    if (final) {
        frame('D', message->bytes, message->size);
        message->size = 0;
    }
    return 1;
}

// One message from the parent. 1 sent, 0 when the connection failed; stdin ending
// ends the helper, because the parent is gone or finished with it.
static int socket_forward(struct channel *channel) {
    char *text = read_line(-1);
    if (!text) _exit(0);
    unsigned long long size = 0;
    int valid = decimal(text, &size) && size <= SOCKET_MESSAGE_LIMIT;
    free(text);
    if (!valid) return fail('E', "Native HTTP socket message is invalid");
    unsigned char *message = malloc(size ? (size_t)size : 1);
    if (!message) return fail('E', "Native HTTP is out of memory");
    if (!read_input(message, (size_t)size)) _exit(0);
    int sent = socket_send(channel, OP_TEXT, message, (size_t)size);
    free(message);
    return sent;
}

// Serves messages in both directions until the connection ends, leaving its reason.
static void socket_loop(struct channel *channel, unsigned long long idle_limit) {
    struct socket_message message = {0};
    long long activity = now_ms();
    for (;;) {
        int buffered = channel->start != channel->end || (channel->ssl && ssl_pending(channel->ssl) > 0);
        int waiting = input_start != input_end;
        struct pollfd ready[2] = { { channel->socket, POLLIN, 0 }, { STDIN_FILENO, POLLIN, 0 } };
        if (!buffered && !waiting) {
            long long left = (long long)idle_limit - (now_ms() - activity);
            if (left <= 0) _exit(0);
            int polled = poll(ready, 2, left > 60000 ? 60000 : (int)left);
            if (polled < 0 && errno != EINTR) { fail('E', "Native HTTP socket wait failed"); return; }
            if (polled <= 0) continue;
        }
        if (buffered || (ready[0].revents & (POLLIN | POLLHUP | POLLERR))) {
            if (!socket_receive(channel, &message)) return;
            activity = now_ms();
        }
        if (waiting || (ready[1].revents & (POLLIN | POLLHUP | POLLERR))) {
            if (!socket_forward(channel)) return;
            activity = now_ms();
        }
    }
}

static int serve_socket(const char *idle_text) {
    unsigned long long idle_limit = 0;
    if (!decimal(idle_text, &idle_limit) || idle_limit == 0) return 2;
    char *url = decoded_line();
    long count = url ? read_header_count() : -1;
    struct header *headers = count >= 0 ? read_headers(count) : NULL;
    struct target target;
    struct channel *channel = NULL;
    if (!headers) {
        fail('E', "Native HTTP request is invalid");
    } else if (parse_target(url, &target)) {
        if (target.proxy && !target.secure) {
            fail('E', "Native HTTP sockets cannot use an http:// proxy for plain connections");
        } else if ((channel = open_channel(&target, CONNECT_LIMIT_MS))) {
            operation_limit_ms = SOCKET_WRITE_LIMIT_MS;
            set_socket_limits(channel, SOCKET_WRITE_LIMIT_MS);
            if (socket_upgrade(channel, &target, headers, count)) {
                frame('A', NULL, 0);
                socket_loop(channel, idle_limit);
            }
        }
    }
    frame('E', failure, strlen(failure));
    close_channel(channel);
    return 1;
}
