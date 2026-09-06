import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import { encodeNode, decodeNode } from "../src/sessions/codec.ts";
import { estimateRequestInputTokens } from "../src/context/budget.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { compactContext } from "../src/context/compactor.ts";
import { measureResponsesInput } from "../src/providers/input-measurement.ts";
import type { Message, RequestInput } from "../src/types.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor, waitForIdle } from "../dev/test-support/app-harness.ts";
import { runApp } from "../src/tui/app.ts";

const policy = policyForContextWindow({ tokens: 64_000 }, 85);
const evidence = "export function checked(value) { return value !== undefined ? value : null; }\n"
  .repeat(230).slice(0, 16_384);
const measure = (input: RequestInput, signal?: AbortSignal) => measureResponsesInput(input, "openai-codex", signal);
function history(reads: number): Message[] {
  const messages: Message[] = [{ role: "user", content: [{ kind: "text", text: "Inspect sources and keep the API stable." }] }];
  for (let i = 0; i < reads; i++) messages.push(
    { role: "assistant", content: [{ kind: "tool_call", id: `old-${i}`, name: "fixture", input: {} }] },
    { role: "user", content: [{ kind: "tool_result", id: `old-${i}`, output: evidence, isError: false }] },
  );
  messages.push({ role: "assistant", content: [{ kind: "text", text: "Inspection complete." }] });
  return messages;
}

test("a restored TUI conversation measures its actual projection without premature compact or tool replay", async () => {
  let summaries = 0;
  let requests = 0;
  let replayed = 0;
  const from = { ...provider(), id: "openai-codex", measureInput: measure,
    contextWindow: async () => ({ tokens: 64_000 }), async send(request: import("../src/types.ts").SendRequest) {
      if (request.identity?.purpose === "compaction") summaries++;
      else requests++;
      return provider("Continued.").send(request);
    } };
  const current = session(from);
  const messages = history(8);
  const tree = ConversationTree.empty().commit({ parentId: 0, createdAt: new Date().toISOString(),
    identity: { providerId: from.id, model: current.model, effort: "high" }, messages, blocks: [] }, "completed");
  const saved = decodeNode(JSON.parse(encodeNode(tree.activeNode!, 1, new Date().toISOString())));
  current.conversation = ConversationTree.restore([saved.node], saved.node.id);
  current.tools = [{ name: "fixture", description: "Inert tool", dangerous: false, concurrency: "shared", input: {},
    async run() { replayed++; return { output: "unexpected" }; } }];
  const input = { model: current.model, effort: "high", system: current.system, tools: [], messages };
  assert.ok(estimateRequestInputTokens(input) > policy.triggerTokens);
  assert.ok(await measure(input) < policy.triggerTokens);
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();
  try {
    feed("continue\r");
    await waitFor(() => current.conversation.activeNodeId === 2 &&
      current.conversation.activeNode?.settlement === "completed", "restored turn");
    await waitForIdle(harness, "restored turn idle");
    assert.equal(requests, 1);
    assert.equal(summaries, 0);
    assert.equal(replayed, 0);
    assert.deepEqual(current.conversation.nodes[0]?.messages, messages);
  } finally { feed("/exit\r"); await running; }
});

test("tokenized long context still compacts, retains tool pairs, and persists a restorable anchor", async () => {
  const messages = history(24).slice(0, -1);
  const before = structuredClone(messages);
  const input = { model: "account-fixture", effort: "high", system: "", tools: [], messages };
  const tokens = await measure(input);
  assert.ok(tokens >= policy.triggerTokens);
  const result = await compactContext({ provider: { ...provider("Sources reviewed. Keep the API stable."), measureInput: measure },
    model: input.model, effort: "high", context: messages, turn: messages, nodeId: 1,
    coveredMessages: 0, lastInputTokens: 0, estimatedInputTokens: tokens, policy });
  assert.ok(result);
  assert.ok(result.estimatedInputTokens < policy.triggerTokens);
  assert.equal(result.messages.at(-2)?.content[0]?.kind, "tool_call");
  assert.equal(result.messages.at(-1)?.content[0]?.kind, "tool_result");
  assert.deepEqual(messages, before);
  const answer: Message = { role: "assistant", content: [{ kind: "text", text: "Done." }] };
  const settled = [...messages, answer];
  const tree = ConversationTree.empty().commit({ parentId: 0, createdAt: new Date().toISOString(),
    identity: { providerId: "openai-codex", model: input.model, effort: "high" },
    messages: settled, blocks: [], context: result.anchor }, "completed");
  const saved = decodeNode(JSON.parse(encodeNode(tree.activeNode!, 1, new Date().toISOString())));
  const restored = ConversationTree.restore([saved.node], 1);
  assert.deepEqual(restored.history, settled);
  assert.deepEqual(restored.contextHistory, [...result.messages, answer]);
});
