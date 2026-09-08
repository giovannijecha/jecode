import { test } from "node:test";
import assert from "node:assert/strict";
import { assembleOpenAI } from "../src/providers/openai-stream.ts";
import { fromWireResponse } from "../src/providers/openai-wire.ts";

for (const type of ["response.completed", "response.done", "response.incomplete"]) {
  test(`${type} does not hide a failed terminal envelope`, async () => {
    async function* events() {
      yield { type: "response.output_item.done", item: { type: "function_call", call_id: "partial",
        name: "write_file", arguments: "{}" } };
      yield { type, response: { status: "failed", output: [], error: { code: "server_error", message: "Failed generation." } } };
    }
    await assert.rejects(assembleOpenAI(events()), /Failed generation/);
  });
}

test("a shorter nonempty final output replaces streamed items, including unissued tool calls", async () => {
  async function* events() {
    yield { type: "response.output_item.done", item: { type: "reasoning", encrypted_content: "fixture" } };
    yield { type: "response.output_item.done", item: { type: "function_call", call_id: "unissued",
      name: "write_file", arguments: "{}" } };
    yield { type: "response.completed", response: { status: "completed", output: [
      { type: "message", content: [{ type: "refusal", refusal: "Declined." }] },
    ] } };
  }
  const result = fromWireResponse(await assembleOpenAI(events()));
  assert.deepEqual(result.content, [{ kind: "text", text: "[refused] Declined." }]);
  assert.equal(result.completion, "refused");
});

for (const response of [
  { status: "cancelled" },
  { status: "in_progress" },
  { status: "completed", error: { code: "server_error" } },
]) {
  test(`invalid final state ${JSON.stringify(response)} cannot be a successful answer`, async () => {
    async function* events() { yield { type: "response.completed", response: { ...response, output: [] } }; }
    await assert.rejects(assembleOpenAI(events()), /Response did not complete/);
  });
}
