// Real Responses wire/measurement code behind an inert loopback transport.
import assert from "node:assert/strict";
import type { Provider, SendRequest } from "../../src/types.ts";
import { measureResponsesInput } from "../../src/providers/input-measurement.ts";
import { ResponsesSession } from "../../src/providers/responses-session.ts";
import { requestResponses } from "../../src/providers/responses-request.ts";
import { fromWireResponse, toWireItems, toWireTool } from "../../src/providers/openai-wire.ts";
import { provider } from "../test-support/app.ts";
import { completed, responsesServer, socketJson } from "../test-support/responses-server.ts";

export const SUMMARY = "Keep the public API stable. Earlier fixture files were inspected; preserve all recorded effects.";

export async function contextFixture(reads: number, termination: "interrupt" | "disconnect") {
  let requests = 0;
  let summaries = 0;
  let waiting = false;
  let resumed = false;
  let streamEvents = 0;
  const measurements: number[] = [];
  const measuredTokens: number[] = [];
  const server = await responsesServer({ message(body, socket) {
    const summary = String(body["instructions"]).includes("durable working memory");
    if (summary) {
      summaries++;
      socketJson(socket, { type: "response.output_text.delta", delta: SUMMARY });
      socketJson(socket, completed(`summary-${summaries}`, SUMMARY));
      return;
    }
    requests++;
    if (resumed) {
      assert.equal(body["previous_response_id"], undefined, "resume must send self-contained context");
      assert.match(JSON.stringify(body["input"]), /Keep the public API stable/);
      socketJson(socket, { type: "response.output_text.delta", delta: "Recovered fixture work." });
      socketJson(socket, completed("recovered", "Recovered fixture work."));
    } else if (requests <= reads) {
      const event = completed(`read-${requests}`);
      socketJson(socket, { type: "response.output_text.delta", delta: `Inspecting fixture ${requests}.` });
      socketJson(socket, { ...event, response: { ...event.response, usage: undefined, output: [
        { type: "function_call", id: `fc_${requests}`, call_id: `call_${requests}`,
          name: "fixture_read", arguments: JSON.stringify({ index: requests - 1 }), status: "completed" },
      ] } });
    } else {
      waiting = true;
      socketJson(socket, { type: "response.created" });
      socketJson(socket, { type: "response.output_text.delta", delta: "Waiting for fixture interruption." });
      // Deliberately unfinished; cancellation must settle the stream without replay.
      if (termination === "disconnect") socket.end();
    }
  } });
  const send = async (request: SendRequest, transport: ResponsesSession) => {
    const result = await requestResponses("openai", server.url, {}, {
      model: request.model, instructions: request.system,
      input: request.messages.flatMap(message => toWireItems(message)),
      tools: request.tools.map(toWireTool), stream: true,
    }, { ...request, onStream(event) { streamEvents++; request.onStream?.(event); } }, transport);
    return fromWireResponse(result);
  };
  const wire: Provider = { ...provider(), id: "openai", defaultModel: "gpt-5",
    contextWindow: async () => ({ tokens: 32_000 }),
    async measureInput(request, signal) {
      const started = performance.now();
      const tokens = await measureResponsesInput(request, "openai", signal);
      measurements.push(performance.now() - started);
      measuredTokens.push(tokens);
      return tokens;
    },
    async send(request) {
      const transport = new ResponsesSession();
      try { return await send(request, transport); } finally { transport.close(); }
    },
    openTurn() {
      const transport = new ResponsesSession();
      return { send: request => send(request, transport), close: () => transport.close() };
    },
  };
  return { wire, measurements, measuredTokens, close: () => server.close(),
    resume() { resumed = true; waiting = false; },
    state: () => ({ requests, summaries, waiting, streamEvents, ...server.counts() }) };
}
