import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { runTurn } from "../src/controller.ts";
import { ResponsesSession } from "../src/providers/responses-session.ts";
import { requestResponses } from "../src/providers/responses-request.ts";
import { fromWireResponse, toWireItems } from "../src/providers/openai-wire.ts";
import { completed, responsesServer, socketJson } from "../dev/test-support/responses-server.ts";
import { echo, events, options, scripted } from "../dev/test-support/controller.ts";

const call = (id: string) => ({ type: "function_call", id: `fc_${id}`, status: "completed",
  call_id: id, name: "effect", arguments: "{}" });
const stages: Record<string, unknown[]> = {
  awaiting: [],
  accepted: [{ type: "response.created" }],
  text: [{ type: "response.output_text.delta", delta: "unfinished" }],
  tool: [{ type: "response.output_item.done", item: call("partial") }],
};

for (const transport of ["http", "websocket"] as const) {
  for (const [stage, partial] of Object.entries(stages)) {
    for (const committed of [false, true]) {
      test(`${transport} loss at ${stage}, prior effect ${committed}: no replay and a complete-context continuation`, async (t) => {
        const sent: Record<string, unknown>[] = [];
        let recovering = false;
        let effects = 0;
        const response = (body: Record<string, unknown>): unknown[] => {
          sent.push(body);
          if (recovering) return [completed("recovered")];
          if (committed && sent.length === 1) {
            const done = completed("committed");
            return [{ ...done, response: { ...done.response, output: [call("saved")] } }];
          }
          return partial;
        };
        const server = await responsesServer({
          message(body, socket) {
            for (const event of response(body)) socketJson(socket, event);
            if (!committed || sent.length > 1) socket.end();
          },
          http(request, res) {
            let body = "";
            request.setEncoding("utf8");
            request.on("data", (chunk: string) => { body += chunk; });
            request.on("end", () => {
              res.writeHead(200, { "content-type": "text/event-stream" });
              for (const event of response(JSON.parse(body) as Record<string, unknown>)) {
                res.write(`data: ${JSON.stringify(event)}\n\n`);
              }
              // EOF without a terminal event is a failed response even after
              // a complete tool item. No HTTP rejection authorizes a retry.
              res.end();
            });
          },
        });
        t.after(() => server.close());
        const send = async (request: SendRequest, session?: ResponsesSession) => fromWireResponse(
          await requestResponses("openai", server.url, {}, { model: "fixture", stream: true,
            input: request.messages.flatMap(message => toWireItems(message)) }, request, session));
        const provider: Provider = { ...scripted([]), send: request => send(request),
          ...(transport === "http" ? {} : { openTurn() {
            const session = new ResponsesSession();
            return { send: (request: SendRequest) => send(request, session), close: () => session.close() };
          } }),
        };
        const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "complete the fixture" }] }];
        const setup = options(provider, { tools: [{ ...echo, name: "effect", concurrency: "exclusive",
          async run() { effects++; return { output: "committed effect" }; } }] });
        await assert.rejects(runTurn(history, setup, events()));
        assert.equal(sent.length, committed ? 2 : 1);
        assert.equal(effects, committed ? 1 : 0);
        assert.doesNotMatch(JSON.stringify(history), /partial|unfinished/);
        assert.equal(history.flatMap(message => message.content).filter(block => block.kind === "tool_result").length,
          committed ? 1 : 0);
        const saved = structuredClone(history);
        recovering = true;
        history.push({ role: "user", content: [{ kind: "text", text: "continue" }] });
        await runTurn(history, setup, events());
        assert.equal(effects, committed ? 1 : 0);
        assert.deepEqual(history.slice(0, saved.length), saved);
        assert.equal(sent.at(-1)?.["previous_response_id"], undefined);
        assert.match(JSON.stringify(sent.at(-1)?.["input"]), /complete the fixture/);
        if (committed) assert.match(JSON.stringify(sent.at(-1)?.["input"]), /committed effect/);
        assert.equal(server.counts()[transport === "http" ? "upgrades" : "posts"], 0);
      });
    }
  }
}
