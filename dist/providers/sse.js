// Server-sent events, read off a fetch response body.
//
// The format is small: `field: value` lines, a blank line ends an event. Only
// `data` matters here — both providers put the event discriminator inside the
// JSON payload, so the `event:` line is redundant and skipped.
import { MAX_SSE_EVENT_CHARS } from "./stream-limits.js";
export async function* readSseJson(body) {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let finished = false;
    try {
        for (;;) {
            const { done, value } = await reader.read();
            if (done)
                break;
            buffer += decoder.decode(value, { stream: true });
            for (;;) {
                const boundary = findBoundary(buffer);
                if (boundary === undefined)
                    break;
                assertEventSize(boundary.start);
                const chunk = buffer.slice(0, boundary.start);
                buffer = buffer.slice(boundary.end);
                const payload = parseData(chunk);
                if (payload !== undefined)
                    yield payload;
            }
            assertEventSize(buffer.length);
        }
        // A stream that ends without a trailing blank line still owes us its last
        // event.
        buffer += decoder.decode();
        assertEventSize(buffer.length);
        const payload = parseData(buffer);
        if (payload !== undefined)
            yield payload;
        finished = true;
    }
    finally {
        if (!finished)
            await reader.cancel().catch(() => undefined);
        reader.releaseLock();
    }
}
function assertEventSize(length) {
    if (length > MAX_SSE_EVENT_CHARS) {
        throw new Error(`SSE event exceeded ${MAX_SSE_EVENT_CHARS} characters`);
    }
}
// Handles both LF and CRLF framing without normalising the buffer first — a
// normalising pass would have to cope with a \r\n split across two chunks.
function findBoundary(buffer) {
    const lf = buffer.indexOf("\n\n");
    const crlf = buffer.indexOf("\r\n\r\n");
    if (lf === -1 && crlf === -1)
        return undefined;
    if (crlf !== -1 && (lf === -1 || crlf < lf))
        return { start: crlf, end: crlf + 4 };
    return { start: lf, end: lf + 2 };
}
function parseData(chunk) {
    const data = chunk
        .split(/\r?\n/)
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice("data:".length).trimStart())
        .join("\n");
    if (data === "" || data === "[DONE]")
        return undefined;
    try {
        return JSON.parse(data);
    }
    catch {
        return undefined;
    }
}
