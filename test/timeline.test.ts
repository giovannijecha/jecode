import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import type { Session } from "../src/session.ts";
import { selectTimeline, timelinePicker } from "../src/timeline.ts";
import type { Message, Provider } from "../src/types.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

const identity = { providerId: "fake", model: "fake-1", effort: "high" };

test("timeline renders linear turns compactly and exposes branch structure", () => {
  const tree = branchedTree();
  const timeline = timelinePicker(tree, STEEL);

  assert.deepEqual(timeline.nodeIds, [1, 2, 4, 3]);
  assert.deepEqual(timeline.picker.options.map((option) => option.label), [
    "• establish the goal",
    "├─ take path A",
    "│  • continue path A",
    "└─ take path B",
  ]);
  assert.equal(timeline.picker.options[3]?.value, "active");
  assert.equal(timeline.picker.index, 3);
  assert.equal(timeline.picker.searchable, true);
});

test("timeline selection changes only the active in-memory path", async () => {
  const current = session(branchedTree());
  current.usage.inputTokens = 999;

  const selected = await selectTimeline(current, () => Promise.resolve(2));

  assert.equal(selected, true);
  assert.equal(current.conversation.activeNodeId, 4);
  assert.deepEqual(texts(current.conversation.history), [
    "establish the goal",
    "goal set",
    "take path A",
    "A selected",
    "continue path A",
    "A complete",
  ]);
  assert.equal(current.conversation.nodes.length, 4);
  assert.equal(current.usage.inputTokens, 0);
});

test("timeline excludes unfinished turns and cancellation is silent", async () => {
  const complete = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity,
    messages: turn("done", "complete"),
    blocks: [],
  }, "completed");
  const pending = complete.commit({
    parentId: 1,
    createdAt: "2026-09-02T10:01:00.000Z",
    identity,
    messages: [
      { role: "user", content: [{ kind: "text", text: "inspect" }] },
      {
        role: "assistant",
        content: [{ kind: "tool_call", id: "call", name: "read_file", input: { path: "a.ts" } }],
      },
      {
        role: "user",
        content: [{ kind: "tool_result", id: "call", output: "content", isError: false }],
      },
    ],
    blocks: [],
  }, "checkpointed");
  const current = session(pending);
  const timeline = timelinePicker(pending, STEEL);

  assert.deepEqual(timeline.nodeIds, [1]);
  assert.equal(timeline.picker.index, 0);
  assert.equal(timeline.picker.options[0]?.value, "active");
  assert.equal(await selectTimeline(current, () => Promise.resolve(undefined)), false);
  assert.equal(current.conversation.activeNodeId, 2);
});

test("timeline keeps a completed descendant while collapsing an unfinished checkpoint", () => {
  const first = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity,
    messages: turn("first", "complete"),
    blocks: [],
  }, "completed");
  const pending = first.commit({
    parentId: 1,
    createdAt: "2026-09-02T10:01:00.000Z",
    identity,
    messages: [
      { role: "user", content: [{ kind: "text", text: "inspect" }] },
      {
        role: "assistant",
        content: [{ kind: "tool_call", id: "call", name: "read_file", input: { path: "a.ts" } }],
      },
      {
        role: "user",
        content: [{ kind: "tool_result", id: "call", output: "content", isError: false }],
      },
    ],
    blocks: [],
  }, "checkpointed");
  const recovered = pending.commit({
    parentId: 2,
    createdAt: "2026-09-02T10:02:00.000Z",
    identity,
    messages: turn("continue", "done"),
    blocks: [],
  }, "completed");

  const timeline = timelinePicker(recovered, STEEL);
  assert.deepEqual(timeline.nodeIds, [1, 3]);
  assert.match(timeline.picker.options[1]?.label ?? "", /• continue/);
  assert.equal(timeline.picker.options[1]?.value, "active");
});

function branchedTree(): ConversationTree {
  const first = append(ConversationTree.empty(), 0, "establish the goal", "goal set", "10:00");
  const pathA = append(first, 1, "take path A", "A selected", "10:01");
  const pathB = append(pathA.select(1), 1, "take path B", "B selected", "10:02");
  return append(pathB.select(2), 2, "continue path A", "A complete", "10:03").select(3);
}

function append(
  tree: ConversationTree,
  parentId: number,
  user: string,
  answer: string,
  time: string,
): ConversationTree {
  return tree.commit({
    parentId,
    createdAt: `2026-09-02T${time}:00.000Z`,
    identity,
    messages: turn(user, answer),
    blocks: [{ kind: "user", text: user }, { kind: "answer", text: answer }],
  }, "completed");
}

function turn(user: string, answer: string): Message[] {
  return [
    { role: "user", content: [{ kind: "text", text: user }] },
    { role: "assistant", content: [{ kind: "text", text: answer }] },
  ];
}

function texts(messages: readonly Message[]): string[] {
  return messages.flatMap((message) => message.content)
    .filter((block) => block.kind === "text")
    .map((block) => block.text);
}

function session(conversation: ConversationTree): Session {
  const provider: Provider = {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    send: () => Promise.reject(new Error("not called")),
  };
  return {
    config: {
      providerId: "fake",
      model: "fake-1",
      reducedMotion: true,
      effort: "high",
      maxTokens: 4_096,
      maxSteps: 8,
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider,
    model: "fake-1",
    palette: STEEL,
    tools: [],
    system: "",
    conversation,
    usage: emptyUsage(),
  };
}
