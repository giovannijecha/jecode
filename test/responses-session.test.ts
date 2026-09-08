import { test } from "node:test";
import assert from "node:assert/strict";
import { ResponsesSession } from "../src/providers/responses-session.ts";
import type { ResponsesBody } from "../src/providers/responses-session.ts";
import { requestResponses } from "../src/providers/responses-request.ts";
import type { SendRequest, TransportObservation } from "../src/types.ts";
import { responsesServer, socketJson, socketText, completed } from "../dev/test-support/responses-server.ts";
import { isContextOverflow } from "../src/context/policy.ts";
import { normalizeProviderError } from "../src/providers/failure.ts";

const headers = { authorization: "Bearer fixture" };
const request: SendRequest = { model: "fixture", system: "instructions", messages: [], tools: [],
  maxTokens: 1000, effort: "high" };
const body = (): ResponsesBody => ({ model: "fixture", instructions: "instructions", tools: [],
  reasoning: { effort: "high", summary: "auto" }, stream: true, store: false,
  include: ["reasoning.encrypted_content"], input: [{ role: "user", content: "hello" }] });

test("authenticated Responses sockets send exact deltas while keeping the complete caller input", async (t) => {
  const sent: Record<string, unknown>[] = [];
  const server = await responsesServer({
    upgrade(req) { assert.equal(req.headers.authorization, headers.authorization); return true; },
    message(value, socket) { sent.push(value); socketJson(socket, completed(`response-${sent.length}`)); },
  });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  const metrics: TransportObservation[] = [];
  const req = { ...request, onTransport: (value: TransportObservation) => metrics.push(value) };
  const first = body();
  const response = await requestResponses("openai", server.url, headers, first, req, session);
  const next = { ...first, input: [...first.input, ...response.output!, { type: "function_call_output", call_id: "call", output: "file" }] };
  const saved = structuredClone(next);
  await requestResponses("openai", server.url, headers, next, req, session);
  assert.deepEqual(next, saved);
  assert.equal(sent[0]?.["type"], "response.create");
  assert.equal(sent[0]?.["stream"], undefined);
  assert.equal(sent[0]?.["store"], false);
  assert.equal(sent[1]?.["previous_response_id"], "response-1");
  assert.deepEqual(sent[1]?.["input"], next.input.slice(-1));
  assert.equal(server.counts().upgrades, 1);
  assert.equal(server.counts().posts, 0);
  assert.equal(metrics.at(-1)?.incremental, true);
  assert.equal(metrics.at(-1)?.reused, true);
  assert.equal(metrics.at(-1)?.receivedEvents, 1);
  assert.ok(metrics.at(-1)!.requestBytes < Buffer.byteLength(JSON.stringify({ type: "response.create", ...next })));
});

test("context replacement and settings changes send full input without a predecessor", async (t) => {
  const sent: Record<string, unknown>[] = [];
  const server = await responsesServer({ message(value, socket) { sent.push(value); socketJson(socket, completed()); } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  let next = body();
  for (const change of ["first", "effort", "tools", "model", "context", "output-budget"]) {
    const result = await requestResponses("openai", server.url, headers, next, request, session);
    assert.equal(sent.at(-1)?.["previous_response_id"], undefined, change);
    next = { ...next, input: [...next.input, ...result.output!, { role: "user", content: change }] };
    if (change === "first") next["reasoning"] = { effort: "low" };
    if (change === "effort") next["tools"] = [{ name: "other" }];
    if (change === "tools") next["model"] = "other";
    if (change === "model") next.input = [{ role: "user", content: "compacted" }];
    if (change === "context") next["max_output_tokens"] = 500;
  }
});

test("a missing incremental predecessor retries once with the complete input", async (t) => {
  const sent: Record<string, unknown>[] = [];
  const server = await responsesServer({ message(value, socket) {
    sent.push(value);
    socketJson(socket, sent.length === 2 ? { type: "error", error: { code: "previous_response_not_found" } } : completed());
  } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  const first = body();
  const result = await requestResponses("openai", server.url, headers, first, request, session);
  const next = { ...first, input: [...first.input, ...result.output!, { role: "user", content: "next" }] };
  await requestResponses("openai", server.url, headers, next, request, session);
  assert.equal(sent.length, 3);
  assert.equal(sent[1]?.["previous_response_id"], "response-fixture");
  assert.equal(sent[2]?.["previous_response_id"], undefined);
  assert.deepEqual(sent[2]?.["input"], next.input);
});

test("a rejected upgrade falls back to HTTP once per turn without sending a socket generation", async (t) => {
  const server = await responsesServer({
    upgrade(_request, socket) { socket.end("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"); return false; },
    message() { assert.fail("upgrade rejection must not generate"); },
    http(_request, response) { response.writeHead(200, { "content-type": "text/event-stream" }); response.end(`data: ${JSON.stringify(completed())}\n\n`); },
  });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  for (let index = 0; index < 2; index++) await requestResponses("openai", server.url, headers, body(), request, session);
  assert.deepEqual(server.counts(), { upgrades: 1, posts: 2 });
});

test("ambiguous streaming failures never fall back or replay generation", async (t) => {
  let generations = 0;
  const server = await responsesServer({ message(_value, socket) {
    generations++;
    socketJson(socket, { type: "response.output_text.delta", delta: "partial" });
    socket.end();
  } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  await assert.rejects(requestResponses("openai", server.url, headers, body(), request, session), /WebSocket/);
  assert.equal(generations, 1);
  assert.equal(server.counts().posts, 0);
});

test("cancellation closes the active socket and no request survives the turn scope", async (t) => {
  const control = new AbortController();
  const server = await responsesServer({ message() { control.abort(new Error("user interrupted")); } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  await assert.rejects(requestResponses("openai", server.url, headers, body(), { ...request, signal: control.signal }, session), /user interrupted/);
  session.close();
  await assert.rejects(requestResponses("openai", server.url, headers, body(), request, session), /scope is not available/);
  assert.equal(server.counts().posts, 0);
});

test("invalid and oversized WebSocket messages fail without HTTP replay", async () => {
  for (const payload of ["{bad", "x".repeat(1_000_001)]) {
    const server = await responsesServer({ message(_value, socket) { socketText(socket, payload); } });
    const session = new ResponsesSession();
    try {
      await assert.rejects(requestResponses("openai", server.url, headers, body(), request, session), (error) => {
        assert.ok(error instanceof Error);
        assert.match(error.message, /WebSocket.*(invalid JSON|limit)/);
        assert.equal(normalizeProviderError("openai", error).kind, "unknown");
        return true;
      });
      assert.equal(server.counts().posts, 0);
    } finally { session.close(); await server.close(); }
  }
});

test("account HTTP fallback replays bounded turn routing state only within the same authorization scope", async (t) => {
  const seen: (string | string[] | undefined)[] = [];
  const server = await responsesServer({
    upgrade(_request, socket) { socket.end("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"); return false; },
    message() { assert.fail("no socket generation"); },
    http(request, response) {
      assert.equal(request.headers["openai-beta"], "responses=experimental");
      seen.push(request.headers["x-codex-turn-state"]);
      response.writeHead(200, { "content-type": "text/event-stream", "x-codex-turn-state": "fixture-routing" });
      response.end(`data: ${JSON.stringify(completed())}\n\n`);
    },
  });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  const accountHeaders = { ...headers, "openai-beta": "responses=experimental" };
  await requestResponses("openai-codex", server.url, accountHeaders, body(), request, session);
  await requestResponses("openai-codex", server.url, accountHeaders, body(), request, session);
  await requestResponses("openai-codex", server.url, { ...accountHeaders, authorization: "Bearer different-fixture" }, body(), request, session);
  assert.deepEqual(seen, [undefined, "fixture-routing", undefined]);
});

test("authentication changes and disconnected sockets rebuild the complete request", async (t) => {
  const sent: Record<string, unknown>[] = [];
  const server = await responsesServer({ message(value, socket) {
    sent.push(value);
    socketJson(socket, completed());
    if (sent.length === 2) socket.end(Buffer.from([0x88, 0]));
  } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  let next = body();
  for (let index = 0; index < 3; index++) {
    const reply = await requestResponses("openai", server.url,
      index === 0 ? headers : { authorization: "Bearer changed-fixture" }, next, request, session);
    assert.equal(sent.at(-1)?.["previous_response_id"], undefined);
    next = { ...next, input: [...next.input, ...reply.output!, { role: "user", content: "next" }] };
    if (index === 1) await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.equal(server.counts().upgrades, 3);
});

test("pre-cancelled requests never open a connection", async (t) => {
  const server = await responsesServer({ message() { assert.fail("must not generate"); } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  const control = new AbortController();
  control.abort(new Error("already interrupted"));
  await assert.rejects(requestResponses("openai", server.url, headers, body(), { ...request, signal: control.signal }, session), /already interrupted/);
  assert.deepEqual(server.counts(), { upgrades: 0, posts: 0 });
});

test("a complete response remains valid when the peer immediately ends its connection", async (t) => {
  const server = await responsesServer({ message(_value, socket) {
    socketJson(socket, completed());
    socket.end();
  } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  const result = await requestResponses("openai", server.url, headers, body(), request, session);
  assert.equal(result.status, "completed");
  assert.equal(server.counts().posts, 0);
});

test("only a pre-stream rejection retains status for context recovery or account refresh", async () => {
  for (const status of [400, 401, 429, 500]) {
    for (const progressed of [false, true]) {
      const server = await responsesServer({ message(_value, socket) {
        if (progressed) socketJson(socket, { type: "response.output_text.delta", delta: "partial" });
        socketJson(socket, { type: "error", status, error: {
          code: "context_length_exceeded", message: "Request rejected" } });
      } });
      const session = new ResponsesSession();
      try {
        await assert.rejects(requestResponses("openai", server.url, headers, body(), request, session), (error) => {
          assert.ok(error instanceof Error);
          assert.equal((error as Error & { status?: number }).status, progressed ? undefined : status);
          assert.equal(isContextOverflow(error), !progressed && status === 400);
          return true;
        });
        assert.deepEqual(server.counts(), { upgrades: 1, posts: 0 });
      } finally { session.close(); await server.close(); }
    }
  }
});
