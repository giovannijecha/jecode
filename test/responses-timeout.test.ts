import { test } from "node:test";
import type { TestContext } from "node:test";
import assert from "node:assert/strict";
import { responsesServer, socketJson, completed } from "../dev/test-support/responses-server.ts";
import { ResponsesSession } from "../src/providers/responses-session.ts";
import { requestResponses } from "../src/providers/responses-request.ts";
import { normalizeProviderError } from "../src/providers/failure.ts";
import { transportFailureDiagnostic } from "../src/providers/transport-error.ts";
import { providerFailure } from "../src/provider-errors.ts";
import { provider } from "../dev/test-support/app.ts";

const flush = () => new Promise<void>(resolve => setImmediate(resolve));

async function waitingStream(t: TestContext, transport: "websocket" | "http", providerId = "openai") {
  // Native loopback I/O stays real; only application timers and their monotonic
  // clock advance. Install before setup so no native deadline is left armed.
  let now = performance.now();
  t.mock.method(performance, "now", () => now);
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let emit!: (value: unknown) => void;
  let started!: () => void;
  const ready = new Promise<void>(resolve => { started = resolve; });
  let generations = 0;
  const server = await responsesServer({
    upgrade(_request, socket) {
      if (transport === "websocket") return true;
      socket.end("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
      return false;
    },
    message(_body, socket) {
      generations++;
      emit = value => socketJson(socket, value);
      emit({ type: "response.output_text.delta", delta: "partial" });
    },
    http(_request, response) {
      generations++;
      response.writeHead(200, { "content-type": "text/event-stream" });
      emit = value => { response.write(`data: ${JSON.stringify(value)}\n\n`); };
      emit({ type: "response.output_text.delta", delta: "partial" });
    },
  });
  const session = new ResponsesSession();
  const events = session.events.bind(session);
  let received: ((event: unknown) => void) | undefined;
  t.mock.method(session, "events", async function* (...args: Parameters<ResponsesSession["events"]>) {
    for await (const event of events(...args)) {
      received?.(event);
      yield event;
    }
  });
  const control = new AbortController();
  t.after(async () => { control.abort(); session.close(); await server.close(); t.mock.timers.reset(); });
  let settled = false;
  const result = requestResponses(providerId, server.url, { authorization: "Bearer fixture" },
    { model: "fixture", input: [], stream: true }, {
      model: "fixture", system: "instructions", messages: [], tools: [], maxTokens: 1000,
      effort: "high", signal: control.signal, onStream() { started(); },
    }, session).then(value => { settled = true; return value; }, error => { settled = true; return error as Error; });
  await ready;
  await flush();
  return {
    async deliver(value: unknown) {
      const delivered = new Promise<void>((resolve, reject) => {
        received = event => {
          try { assert.deepEqual(event, value); resolve(); }
          catch (error) { reject(error); }
        };
      });
      emit(value);
      try {
        await Promise.race([delivered, result.then(value => {
          throw value instanceof Error ? value : new Error("stream ended before fixture delivery");
        })]);
      } finally { received = undefined; }
      // A loopback write or one event-loop turn does not prove client receipt.
      // Observe the parsed event, including keepalives, before advancing time.
      await flush();
    },
    async tick(milliseconds: number) { now += milliseconds; t.mock.timers.tick(milliseconds); await flush(); },
    settled: () => settled,
    generations: () => generations,
    counts: server.counts,
    control,
    result,
  };
}

for (const transport of ["websocket", "http"] as const) {
  for (const providerId of ["openai", "openai-codex"]) {
    test(`${providerId} ${transport} accepts sparse reasoning without a two-minute cutoff`, async t => {
      const stream = await waitingStream(t, transport, providerId);
      // Arm a new application deadline under the controlled clock.
      await stream.deliver({ type: "response.reasoning_summary_text.delta", delta: "thinking" });
      await stream.tick(150_000);
      assert.equal(stream.settled(), false, "a sparse reasoning gap is not a failed connection");
      await stream.deliver({ type: "response.reasoning_summary_text.delta", delta: "more thinking" });
      await stream.tick(150_000);
      assert.equal(stream.settled(), false, "substantive progress renews the deadline");
      await stream.deliver(completed());
      const result = await stream.result;
      assert.ok(!(result instanceof Error), String(result));
      assert.equal(result.status, "completed");
      assert.equal(stream.generations(), 1);
    });
  }

  test(`${transport} keepalives cannot extend the model progress deadline or authorize replay`, async t => {
    const stream = await waitingStream(t, transport);
    await stream.deliver({ type: "response.reasoning_summary_text.delta", delta: "thinking" });
    for (let index = 0; index < 2; index++) {
      await stream.tick(100_000);
      await stream.deliver({ type: "response.in_progress" });
    }
    await stream.tick(99_999);
    assert.equal(stream.settled(), false);
    await stream.tick(1);
    const result = await stream.result;
    assert.ok(result instanceof Error);
    const failure = normalizeProviderError("openai", result);
    assert.deepEqual(transportFailureDiagnostic(failure), { transportFailure: "progress-timeout" });
    assert.match(providerFailure(provider(), failure), /model stream timed out/);
    assert.doesNotMatch(providerFailure(provider(), failure), /check the connection/);
    assert.equal(stream.generations(), 1);
    assert.deepEqual(stream.counts(), { upgrades: 1, posts: transport === "http" ? 1 : 0 });
  });

  test(`${transport} cancellation during sparse reasoning stops without waiting or replay`, async t => {
    const stream = await waitingStream(t, transport);
    await stream.deliver({ type: "response.reasoning_summary_text.delta", delta: "thinking" });
    await stream.tick(150_000);
    assert.equal(stream.settled(), false);
    const interrupted = new Error("user interrupted sparse reasoning");
    stream.control.abort(interrupted);
    assert.equal(await stream.result, interrupted);
    assert.equal(stream.generations(), 1);
  });
}
