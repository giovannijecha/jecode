import { test } from "node:test";
import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import { readSseJson } from "../src/providers/sse.ts";
import { postSse } from "../src/providers/http.ts";
import { assembleOpenAI, openAITerminalEvent } from "../src/providers/openai-stream.ts";
import { responsesServer, completed } from "../dev/test-support/responses-server.ts";

test("a protocol terminator settles before HTTP EOF and releases the tail without cancellation", async () => {
  let control!: ReadableStreamDefaultController<Uint8Array>;
  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({
    start(value) { control = value; value.enqueue(Buffer.from(`data: ${JSON.stringify(completed())}\n\n`)); },
    cancel() { cancelled = true; },
  });
  const events = readSseJson(body, 100_000, undefined, { terminal: openAITerminalEvent });
  const response = await assembleOpenAI(events);
  assert.equal(response.status, "completed");
  assert.equal(cancelled, false);
  control.close();
  await delay(0);
  assert.equal(cancelled, false);
  assert.equal(body.locked, false);
});

test("DONE settles Ollama-style streams after usage without waiting for body close", async () => {
  let control!: ReadableStreamDefaultController<Uint8Array>;
  const body = new ReadableStream<Uint8Array>({ start(value) {
    control = value;
    value.enqueue(Buffer.from('data: {"usage":{"total_tokens":100}}\n\ndata: [DONE]\n\n'));
  } });
  const out = [];
  for await (const event of readSseJson(body, 100_000, undefined, { doneMarker: true })) out.push(event);
  assert.deepEqual(out, [{ usage: { total_tokens: 100 } }]);
  control.close();
  await delay(0);
  assert.equal(body.locked, false);
});

test("bounded tail draining cancels a hanging body, excessive data, or user interruption", async () => {
  for (const mode of ["idle", "oversized", "abort"] as const) {
    let cancelled = false;
    const signal = new AbortController();
    const body = new ReadableStream<Uint8Array>({ start(value) {
      value.enqueue(Buffer.from("data: [DONE]\n\n"));
      if (mode === "oversized") value.enqueue(Buffer.alloc(65_537));
    }, cancel() { cancelled = true; } });
    for await (const _event of readSseJson(body, 100_000, undefined, { doneMarker: true, signal: signal.signal })) { /* terminator only */ }
    if (mode === "abort") signal.abort();
    await delay(mode === "idle" ? 300 : 0);
    assert.equal(cancelled, true, mode);
    assert.equal(body.locked, false, mode);
  }
});

test("returning early without a protocol terminator cancels immediately", async () => {
  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({ start(value) {
    value.enqueue(Buffer.from('data: {"type":"response.created"}\n\n'));
  }, cancel() { cancelled = true; } });
  const events = readSseJson(body, 100_000, undefined, { terminal: openAITerminalEvent });
  await events.next();
  await events.return(undefined);
  assert.equal(cancelled, true);
  assert.equal(body.locked, false);
});

test("completed HTTP generations reuse their connection when EOF trails the terminal event", async (t) => {
  const sockets = new Set<unknown>();
  const server = await responsesServer({ message() {}, http(request, response) {
    sockets.add(request.socket);
    response.writeHead(200, { "content-type": "text/event-stream" });
    response.write(`data: ${JSON.stringify(completed())}\n\n`);
    setTimeout(() => response.end(), 10);
  } });
  t.after(() => server.close());
  for (let index = 0; index < 4; index++) {
    const events = await postSse(server.url, {}, {}, 1000, undefined, undefined, undefined, undefined,
      { terminal: openAITerminalEvent });
    await assembleOpenAI(events);
    await delay(30);
  }
  // Fetch can allocate a second connection while an earlier tail drains. The
  // regression was discarding every connection, not the pool's exact size.
  assert.ok(sockets.size < 4, `${sockets.size} sockets used for four sequential generations`);
});
