import { test } from "node:test";
import assert from "node:assert/strict";
import { runTurn } from "../src/controller.ts";
import type { Message } from "../src/types.ts";
import { assistantText, destroy, echo, events, options, scripted } from "../dev/test-support/controller.ts";

test("live revocation after approval prevents execution and remains visible to the model", async () => {
  const provider = scripted([
    { role: "assistant", content: [{ kind: "tool_call", name: "destroy", id: "call", input: {} }] },
    assistantText("Stopped."),
  ]);
  let allowed = true;
  let executions = 0;
  const sink = events();
  sink.approve = async () => { allowed = false; return true; };
  const history: Message[] = [];
  await runTurn(history, options(provider, {
    toolAllowed: () => allowed,
    tools: [{ ...destroy, async run() { executions++; return { output: "unexpected" }; } }],
  }), sink);
  assert.equal(executions, 0);
  assert.match(JSON.stringify(provider.seen[1]?.messages), /denied by the current session permissions/);
});

test("denied tools do not preview or ask even when advertised in the original request", async () => {
  const provider = scripted([
    { role: "assistant", content: [
      { kind: "tool_call", name: "destroy", id: "write", input: {} },
      { kind: "tool_call", name: "echo", id: "read", input: { text: "data" } },
    ] },
    assistantText("Stopped."),
  ]);
  const sink = events();
  sink.approve = async () => { assert.fail("a denied tool must not ask"); };
  await runTurn([], options(provider, {
    toolAllowed: () => false,
    tools: [destroy, echo].map((tool) => ({
      ...tool,
      async preview() { assert.fail("a denied tool must not preview"); },
      async run() { assert.fail("a denied tool must not execute"); },
    })),
  }), sink);
  const results = provider.seen[1]?.messages.flatMap((message) => message.content)
    .filter((block) => block.kind === "tool_result");
  assert.equal(results?.length, 2);
  assert.ok(results?.every((block) => block.isError && block.output.includes("denied")));
});
