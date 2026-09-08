import { test } from "node:test";
import assert from "node:assert/strict";
import { channel } from "node:diagnostics_channel";
import type { Duplex } from "node:stream";
import { responsesServer, socketJson, socketText, completed } from "../dev/test-support/responses-server.ts";
import { provider } from "../dev/test-support/app.ts";
import { requestResponses } from "../src/providers/responses-request.ts";
import { ResponsesSession } from "../src/providers/responses-session.ts";
import { normalizeProviderError } from "../src/providers/failure.ts";
import { TransportError, transportFailureDiagnostic } from "../src/providers/transport-error.ts";
import { providerFailure } from "../src/provider-errors.ts";
import { CONTEXT_DIAGNOSTIC_CHANNEL, safeDiagnostic } from "../src/context/diagnostics.ts";
import type { RequestDiagnostic } from "../src/context/diagnostics.ts";
import { sendObserved } from "../src/context/request-observation.ts";
import { inputMeter } from "../src/context/measurement.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import type { SendRequest } from "../src/types.ts";

test("live socket failures preserve received counts and distinguish peer closure from local rejection", async () => {
  for (const failure of ["closed", "event-limit", "invalid-json", "native-frame"] as const) {
    const records: RequestDiagnostic[] = [];
    const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
    const receive = (value: unknown) => {
      const event = safeDiagnostic(value);
      if (event?.kind === "request") records.push(event);
    };
    source.subscribe(receive);
    let peer: Duplex | undefined;
    let sent = 0;
    const server = await responsesServer({ message(_body, socket) {
      sent++;
      peer = socket;
      socketJson(socket, { type: "response.reasoning_summary_text.delta", delta: "private thought" });
    } });
    const session = new ResponsesSession();
    const p = { ...provider(), async send(req: SendRequest) {
      try {
        await requestResponses("openai", server.url, { authorization: "private token" },
          { input: [], model: "fixture", stream: true }, { ...req, onStream(event) {
            req.onStream?.(event);
            if (failure === "event-limit") socketText(peer!, "x".repeat(1_000_001));
            else if (failure === "invalid-json") socketText(peer!, "{private malformed payload");
            else if (failure === "native-frame") peer!.write(Buffer.from([0x83, 0])); // Reserved opcode.
            else {
              const reason = Buffer.from("private close reason");
              const frame = Buffer.alloc(4 + reason.length);
              frame[0] = 0x88; frame[1] = 2 + reason.length;
              frame.writeUInt16BE(1008, 2); reason.copy(frame, 4);
              peer!.end(frame);
            }
          } }, session);
        throw new Error("unexpected completion");
      } catch (error) { throw normalizeProviderError("openai", error); }
    } };
    try {
      const req = { model: "fixture", effort: "high", system: "private instructions", messages: [], tools: [], maxTokens: 1024 };
      const measured = await inputMeter(p).measure(req);
      await assert.rejects(sendObserved(p, req, measured, policyForContextWindow({ tokens: 64_000 }, 85), 0, 0), error => {
        assert.ok(error instanceof Error);
        const network = failure === "closed" || failure === "native-frame";
        assert.equal(normalizeProviderError("openai", error).kind, network ? "network" : "unknown");
        if (!network) assert.doesNotMatch(providerFailure(p, error), /check the connection/);
        return true;
      });
      const result = records[0]!;
      assert.equal(result.outcome, "failed");
      assert.equal(result.transport, "websocket");
      assert.equal(result.transportFailure, failure === "native-frame" ? "closed" : failure);
      assert.equal(result.responseStage, "output");
      assert.equal(result.webSocketCloseCode, failure === "closed" ? 1008 : undefined);
      assert.ok(result.firstThinkingMs! >= 0);
      assert.equal(result.receivedEvents, failure === "closed" || failure === "native-frame" ? 1 : 2);
      assert.equal(result.nativeWebSocketError, failure === "native-frame");
      assert.ok(result.connectionAgeMs! >= 0);
      assert.ok(result.lastMessageAgeMs! >= 0);
      assert.ok(result.connectionAgeMs! >= result.lastMessageAgeMs!);
      assert.ok(result.receivedChars! > 0);
      if (failure === "event-limit") assert.equal(result.largestEventChars, 1_000_001);
      assert.doesNotMatch(JSON.stringify(result), /private|payload|authorization|reason"/);
      assert.equal(sent, 1);
      assert.equal(server.counts().posts, 0);
      for (const invalid of [{ transportFailure: "private" }, { webSocketCloseCode: 999 },
        { webSocketCloseCode: 5000 }, { receivedChars: -1 }, { largestEventChars: NaN },
        { responseStage: "private peer event" }, { connectionAgeMs: -1 }, { lastMessageAgeMs: Infinity },
        { nativeWebSocketError: "private error" }, { socketReadEnded: "private address" }]) {
        assert.equal(safeDiagnostic({ ...result, ...invalid }), undefined);
      }
    } finally { source.unsubscribe(receive); session.close(); await server.close(); }
  }
});

test("socket diagnostics distinguish acknowledgement and opaque output without exposing event data", async () => {
  const cases = [
    { stage: "awaiting", events: [] },
    { stage: "awaiting", events: [{ type: "private peer event", token: "private token" }] },
    { stage: "accepted", events: [{ type: "response.created" }, { type: "response.in_progress" }] },
    { stage: "output", events: [
      { type: "response.output_item.added", item: { type: "reasoning", encrypted_content: "private reasoning" } },
      { type: "response.in_progress" },
    ] },
    { stage: "terminal", events: [completed()] },
  ];
  for (const { stage, events } of cases) {
    const records: RequestDiagnostic[] = [];
    const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
    const receive = (value: unknown) => {
      const event = safeDiagnostic(value);
      if (event?.kind === "request") records.push(event);
    };
    source.subscribe(receive);
    let sent = 0;
    const server = await responsesServer({ message(_body, socket) {
      sent++;
      for (const event of events) socketJson(socket, event);
      socket.end();
    } });
    const session = new ResponsesSession();
    const p = { ...provider(), async send(req: SendRequest) {
      await requestResponses("openai", server.url, { authorization: "private token" },
        { input: [], model: "fixture", stream: true }, req, session);
      return { role: "assistant" as const, content: [] };
    } };
    try {
      const req = { model: "fixture", effort: "high", system: "private", messages: [], tools: [], maxTokens: 1024 };
      const measured = await inputMeter(p).measure(req);
      const result = sendObserved(p, req, measured, policyForContextWindow({ tokens: 64_000 }, 85), 0, 0);
      if (stage === "terminal") await result;
      else await assert.rejects(result, /WebSocket/);
      assert.equal(records.length, 1);
      assert.equal(records[0]?.responseStage, stage);
      assert.equal(records[0]?.outcome, stage === "terminal" ? "completed" : "failed");
      assert.equal(records[0]?.firstEventMs, undefined);
      if (stage !== "terminal") {
        assert.equal(records[0]?.socketReadEnded, true);
        // Native Node versions may also emit an error for an unframed EOF.
        assert.equal(typeof records[0]?.nativeWebSocketError, "boolean");
      }
      assert.equal(records[0]?.lastMessageAgeMs === undefined, events.length === 0);
      assert.doesNotMatch(JSON.stringify(records), /private|encrypted_content|peer event/);
      assert.equal(sent, 1);
      assert.equal(server.counts().posts, 0);
    } finally { source.unsubscribe(receive); session.close(); await server.close(); }
  }
});

test("response stage resets on a reused connection before any new response arrives", async (t) => {
  let sent = 0;
  const server = await responsesServer({ message(_body, socket) {
    if (++sent === 1) socketJson(socket, completed());
    else socket.end();
  } });
  const session = new ResponsesSession();
  t.after(async () => { session.close(); await server.close(); });
  const stages: unknown[] = [];
  const observations: import("../src/types.ts").TransportObservation[] = [];
  const req: SendRequest = { model: "fixture", effort: "high", system: "", messages: [], tools: [], maxTokens: 1024,
    onTransport: (event) => { stages.push(event.responseStage); observations.push(event); } };
  const body = { input: [], model: "fixture", stream: true };
  await requestResponses("openai", server.url, {}, body, req, session);
  await assert.rejects(requestResponses("openai", server.url, {}, body, req, session), /WebSocket/);
  assert.deepEqual(stages, ["awaiting", "terminal", "awaiting", "awaiting"]);
  assert.ok(observations[1]!.lastMessageAgeMs! >= 0);
  assert.equal(observations[3]!.lastMessageAgeMs, undefined);
  assert.ok(observations[3]!.connectionAgeMs! >= observations[1]!.connectionAgeMs!);
  assert.deepEqual(server.counts(), { upgrades: 1, posts: 0 });
});

test("transport cause inspection is bounded and preserves native diagnostics through wrappers", () => {
  const failure = new TransportError("connection", "WebSocket connection failed", { cause: new Error("private") });
  assert.deepEqual(transportFailureDiagnostic(normalizeProviderError("openai", failure)), { transportFailure: "connection" });
  const cycle = new Error("private");
  cycle.cause = cycle;
  assert.deepEqual(transportFailureDiagnostic(cycle), {});
  assert.deepEqual(transportFailureDiagnostic(undefined), {});
});
