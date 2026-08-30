import { test } from "node:test";
import assert from "node:assert/strict";
import { readSseJson } from "../src/providers/sse.ts";
import {
  MAX_SSE_EVENT_CHARS,
  MAX_SSE_STREAM_CHARS,
} from "../src/providers/stream-limits.ts";

// Feeds the parser exactly the chunk boundaries given, so a test can put a
// split anywhere — including in the middle of an event's framing.
function stream(...chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      controller.close();
    },
  });
}

async function collect(body: ReadableStream<Uint8Array>): Promise<unknown[]> {
  const out: unknown[] = [];
  for await (const event of readSseJson(body)) out.push(event);
  return out;
}

test("reads consecutive LF-framed events", async () => {
  const events = await collect(
    stream('data: {"type":"a"}\n\ndata: {"type":"b"}\n\n'),
  );
  assert.deepEqual(events, [{ type: "a" }, { type: "b" }]);
});

test("reads CRLF-framed events", async () => {
  const events = await collect(
    stream('data: {"type":"a"}\r\n\r\ndata: {"type":"b"}\r\n\r\n'),
  );
  assert.deepEqual(events, [{ type: "a" }, { type: "b" }]);
});

test("ignores the event: line and joins multi-line data", async () => {
  const events = await collect(stream('event: thing\ndata: {"a":\ndata: 1}\n\n'));
  assert.deepEqual(events, [{ a: 1 }]);
});

test("survives an event split across chunk boundaries", async () => {
  const events = await collect(stream('data: {"ty', 'pe":"a"}\n', '\ndata: {"type":"b"}\n\n'));
  assert.deepEqual(events, [{ type: "a" }, { type: "b" }]);
});

test("survives a CRLF boundary split between the \r and the \n", async () => {
  const events = await collect(stream('data: {"type":"a"}\r\n\r', '\ndata: {"type":"b"}\r\n\r\n'));
  assert.deepEqual(events, [{ type: "a" }, { type: "b" }]);
});

test("skips [DONE] and unparseable payloads without stopping", async () => {
  const events = await collect(
    stream('data: [DONE]\n\ndata: not json\n\ndata: {"type":"a"}\n\n'),
  );
  assert.deepEqual(events, [{ type: "a" }]);
});

test("yields a final event that arrives without a trailing blank line", async () => {
  assert.deepEqual(await collect(stream('data: {"type":"a"}')), [{ type: "a" }]);
});

test("rejects and cancels an event that exceeds the buffer limit", async () => {
  const encoder = new TextEncoder();
  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(`data: ${"x".repeat(MAX_SSE_EVENT_CHARS + 1)}`));
    },
    cancel() {
      cancelled = true;
    },
  });

  await assert.rejects(collect(body), /SSE event exceeded/);
  assert.equal(cancelled, true);
});

test("rejects and cancels an aggregate stream that exceeds the response budget", async () => {
  const encoder = new TextEncoder();
  const payload = `data: {"text":"${"x".repeat(850_000)}"}\n\n`;
  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (let total = 0; total <= MAX_SSE_STREAM_CHARS; total += payload.length) {
        controller.enqueue(encoder.encode(payload));
      }
    },
    cancel() {
      cancelled = true;
    },
  });

  await assert.rejects(collect(body), /SSE stream exceeded/);
  assert.equal(cancelled, true);
});

test("decodes multi-byte characters split across chunks", async () => {
  const encoder = new TextEncoder();
  const bytes = encoder.encode('data: {"text":"è"}\n\n');
  const split = 15; // lands inside the two-byte è
  const events = await collect(
    new ReadableStream({
      start(controller) {
        controller.enqueue(bytes.slice(0, split));
        controller.enqueue(bytes.slice(split));
        controller.close();
      },
    }),
  );
  assert.deepEqual(events, [{ text: "è" }]);
});
