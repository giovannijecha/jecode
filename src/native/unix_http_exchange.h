// Owned HTTP/1.1 framing and replay policy shared by Linux and macOS.
struct exchange {
    char *url, *method;
    struct header *headers;
    long header_count;
    unsigned long long timeout_ms;
    unsigned char *body;
    size_t body_size;
};

static void free_exchange(struct exchange *exchange) {
    free(exchange->url);
    free(exchange->method);
    if (exchange->headers) free_headers(exchange->headers, exchange->header_count);
    free(exchange->body);
    memset(exchange, 0, sizeof(*exchange));
}

// 1 with a whole request, 0 at the end of stdin; -1 when it is invalid.
static int read_exchange(struct exchange *exchange) {
    char *first = read_line(IDLE_LIMIT_MS);
    if (!first) return 0;
    exchange->url = decoded_text(first);
    if (!exchange->url) return -1;
    exchange->method = decoded_line();
    char *timeout = read_line(-1);
    int timed = wire_decimal(timeout, &exchange->timeout_ms);
    free(timeout);
    exchange->header_count = read_header_count();
    if (!http_token(exchange->method) || !timed || exchange->header_count < 0) return -1;
    exchange->headers = read_headers(exchange->header_count);
    if (!exchange->headers) return -1;
    char *length = read_line(-1);
    unsigned long long body_size = 0;
    int sized = wire_decimal(length, &body_size) && body_size < SIZE_MAX;
    free(length);
    if (!sized) return -1;
    exchange->body_size = (size_t)body_size;
    exchange->body = malloc(exchange->body_size ? exchange->body_size : 1);
    if (!exchange->body || !read_input(exchange->body, exchange->body_size)) return -1;
    return 1;
}

static int send_request(struct channel *channel, const struct exchange *exchange, const struct target *target) {
    size_t capacity = 4096, used = 0;
    for (long i = 0; i < exchange->header_count; i++)
        capacity += strlen(exchange->headers[i].name) + strlen(exchange->headers[i].value) + 4;
#ifdef __APPLE__
    if (target->proxy && !target->secure && target->proxy_authorization)
        capacity += strlen(target->proxy_authorization) + 32;
#endif
    capacity += strlen(exchange->method) + strlen(exchange->url) + target->authority_length;
    char *head = malloc(capacity);
    if (!head) return fail('E', "Native HTTP is out of memory");
    const char *leading_slash = *target->path == '?' ? "/" : "";
    used += (size_t)snprintf(head + used, capacity - used, "%s %s%s HTTP/1.1\r\nHost: %.*s\r\n", exchange->method,
        target->proxy && !target->secure ? exchange->url : leading_slash,
        target->proxy && !target->secure ? "" : target->path,
        (int)target->authority_length, target->authority);
    int has_length = 0;
#ifdef __APPLE__
    if (target->proxy && !target->secure && target->proxy_authorization)
        used += (size_t)snprintf(head + used, capacity - used, "Proxy-Authorization: %s\r\n",
            target->proxy_authorization);
#endif
    for (long i = 0; i < exchange->header_count; i++) {
        if (!strcasecmp(exchange->headers[i].name, "Content-Length")) has_length = 1;
        used += (size_t)snprintf(head + used, capacity - used, "%s: %s\r\n",
            exchange->headers[i].name, exchange->headers[i].value);
    }
    if (!has_length && (exchange->body_size || !strcasecmp(exchange->method, "POST") ||
        !strcasecmp(exchange->method, "PUT") || !strcasecmp(exchange->method, "PATCH")))
        used += (size_t)snprintf(head + used, capacity - used, "Content-Length: %zu\r\n", exchange->body_size);
    used += (size_t)snprintf(head + used, capacity - used, "\r\n");
    int sent = used < capacity && send_bytes(channel, head, used);
    free(head);
    if (!used || used >= capacity) return fail('E', "Native HTTP request is invalid");
    return sent && (!exchange->body_size || send_bytes(channel, exchange->body, exchange->body_size));
}

static int copy_body(struct channel *channel, uint64_t left) {
    unsigned char block[16384];
    while (left) {
        size_t want = left > sizeof(block) ? sizeof(block) : (size_t)left;
        int count = receive_bytes(channel, block, want);
        if (count <= 0) return fail('T', "Native HTTP response is incomplete");
        frame('D', block, (size_t)count);
        left -= (uint64_t)count;
    }
    return 1;
}

static int copy_chunks(struct channel *channel, char *line, size_t capacity) {
    for (;;) {
        if (!receive_line(channel, line, capacity)) return fail('T', "Native HTTP response is incomplete");
        char *end = NULL;
        if (!((*line >= '0' && *line <= '9') || (*line >= 'a' && *line <= 'f') || (*line >= 'A' && *line <= 'F')))
            return fail('E', "Native HTTP chunk is invalid");
        errno = 0;
        unsigned long long chunk = strtoull(line, &end, 16);
        if (errno == ERANGE) return fail('E', "Native HTTP chunk is invalid");
        if (*end == ';') {
            size_t extension = strlen(end);
            if (extension < 3 || strcmp(end + extension - 2, "\r\n") != 0) return fail('E', "Native HTTP chunk is invalid");
            for (size_t i = 1; i + 2 < extension; i++) {
                if ((unsigned char)end[i] < 0x20 || (unsigned char)end[i] > 0x7e) return fail('E', "Native HTTP chunk is invalid");
            }
        } else if (strcmp(end, "\r\n") != 0) return fail('E', "Native HTTP chunk is invalid");
        if (chunk == 0) {
            do {
                if (!receive_line(channel, line, capacity)) return fail('T', "Native HTTP response is incomplete");
            } while (strcmp(line, "\r\n") != 0);
            return 1;
        }
        if (!copy_body(channel, chunk)) return 0;
        char ending[2];
        if (!receive_exact(channel, ending, 2) || memcmp(ending, "\r\n", 2)) return fail('T', "Native HTTP response is incomplete");
    }
}

struct head { char *text; size_t size; int status, chunked, has_length, closes; uint64_t length; };

// The final response head, past interim ones. `empty` tells a closed connection
// from a broken response, which matters on a reused connection.
static int read_head(struct channel *channel, struct head *head, char *line, size_t capacity, int *empty) {
    *empty = 1;
    for (;;) {
        size_t used = 0;
        head->chunked = head->has_length = 0;
        head->length = 0;
        if (!receive_line(channel, line, capacity)) return fail('T', "Native HTTP response ended before its headers");
        *empty = 0;
        if (sscanf(line, "HTTP/%*s %d", &head->status) != 1 || head->status < 100 || head->status > 599)
            return fail('T', "Native HTTP response status is invalid");
        head->closes = strncmp(line, "HTTP/1.1", 8) != 0;
        do {
            size_t n = strlen(line);
            if (used + n > 65536) return fail('E', "Native HTTP response headers are too large");
            memcpy(head->text + used, line, n);
            used += n;
            if (strcmp(line, "\r\n") == 0) break;
            if (!strncasecmp(line, "Transfer-Encoding:", 18)) {
                char *value = line + 18;
                while (*value == ' ' || *value == '\t') value++;
                if (strncasecmp(value, "chunked", 7)) return fail('E', "Native HTTP response framing is invalid");
                value += 7;
                while (*value == ' ' || *value == '\t') value++;
                if (strcmp(value, "\r\n") != 0 || head->chunked) return fail('E', "Native HTTP response framing is invalid");
                head->chunked = 1;
            }
            if (!strncasecmp(line, "Content-Length:", 15)) {
                char *value = line + 15, *end = NULL;
                while (*value == ' ' || *value == '\t') value++;
                if (*value < '0' || *value > '9') return fail('E', "Native HTTP response length is invalid");
                errno = 0;
                unsigned long long parsed = strtoull(value, &end, 10);
                while (*end == ' ' || *end == '\t') end++;
                if (errno == ERANGE || strcmp(end, "\r\n") != 0 || (head->has_length && head->length != parsed))
                    return fail('E', "Native HTTP response length is invalid");
                head->length = parsed;
                head->has_length = 1;
            }
            if (!strncasecmp(line, "Connection:", 11) && strcasestr(line + 11, "close")) head->closes = 1;
        } while (receive_line(channel, line, capacity));
        if (used < 4 || memcmp(head->text + used - 4, "\r\n\r\n", 4) != 0 || (head->chunked && head->has_length))
            return fail('T', "Native HTTP response headers are incomplete");
        if (head->status < 200) {
            if (head->status == 101) return fail('E', "Native HTTP response switched protocols unexpectedly");
            continue;
        }
        head->size = used;
        return 1;
    }
}

// Copies one response. `reusable` says the connection ended exactly at its end.
static int copy_response(struct channel *channel, const struct exchange *exchange, const struct head *head, char *line, size_t capacity, int *reusable) {
    frame('D', head->text, head->size);
    if (!strcasecmp(exchange->method, "HEAD") || head->status == 204 || head->status == 304) {
        *reusable = !head->closes;
        return 1;
    }
    if (head->chunked) {
        if (!copy_chunks(channel, line, capacity)) return 0;
        *reusable = !head->closes;
        return 1;
    }
    if (head->has_length) {
        if (!copy_body(channel, head->length)) return 0;
        *reusable = !head->closes;
        return 1;
    }
    unsigned char block[16384];
    for (;;) {
        int count = receive_bytes(channel, block, sizeof(block));
        if (count == 0) return 1;
        if (count < 0) return fail('T', "Native HTTP response is incomplete");
        frame('D', block, (size_t)count);
    }
}

static int safe_to_repeat(const char *method) {
    return !strcasecmp(method, "GET") || !strcasecmp(method, "HEAD");
}

static int perform(const struct exchange *exchange) {
    struct target target = {0};
    int result = 0;
    char *line = malloc(65536);
    struct head head = { .text = malloc(65536) };
    if (!line || !head.text || !parse_target(exchange->url, &target)) goto done;
    unsigned long long timeout = exchange->timeout_ms;
    int connect_ms = timeout && timeout < CONNECT_LIMIT_MS ? (int)timeout : CONNECT_LIMIT_MS;
    // Without a total limit, the per-operation limit still bounds silence.
    operation_limit_ms = timeout && timeout < RECEIVE_LIMIT_MS ? (int)timeout : RECEIVE_LIMIT_MS;
    for (int attempt = 0;; attempt++) {
        struct channel *channel = take_cached(target.key);
        int reused = channel != NULL;
        if (!channel) channel = open_channel(&target, connect_ms);
        if (!channel) break;
        set_socket_limits(channel, operation_limit_ms);
        int empty = 1, reusable = 0;
        int sent = send_request(channel, exchange, &target);
        if (sent && read_head(channel, &head, line, 65536, &empty)) {
            result = copy_response(channel, exchange, &head, line, 65536, &reusable);
            if (result && reusable) keep_channel(channel);
            else close_channel(channel);
            break;
        }
        close_channel(channel);
        // A failed receive does not prove that the server did not process a
        // request, so only safe methods may be repeated on a new connection.
        if (!(reused && empty && attempt < STALE_RETRIES && safe_to_repeat(exchange->method))) break;
    }
done:
    free_target(&target);
    free(line);
    free(head.text);
    return result;
}

static int serve_requests(void) {
    for (;;) {
        struct exchange exchange = {0};
        failure_tag = 'E';
        failure = "Native HTTP request is invalid";
        int state = read_exchange(&exchange);
        if (state == 0) { free_exchange(&exchange); return 0; }
        frame('A', NULL, 0);
        if (state < 0) {
            frame('E', failure, strlen(failure));
            return 1;
        }
        failure = "Native HTTP request failed";
        if (perform(&exchange)) frame('E', NULL, 0);
        else frame(failure_tag, failure, strlen(failure));
        free_exchange(&exchange);
    }
}
