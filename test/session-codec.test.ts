import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import {
  decodeHead,
  decodeMeta,
  decodeNode,
  encodeHead,
  encodeMeta,
  encodeNode,
  SESSION_FILE_LIMITS,
} from "../src/sessions/codec.ts";

test("session node codec round-trips normalized history without provider raw data", () => {
  const tree = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "openai-codex", model: "gpt-5.6-terra", effort: "medium" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "change it" }] },
      {
        role: "assistant",
        content: [{ kind: "tool_call", id: "call-1", name: "edit_file", input: { path: "a.ts", old: "", new: "$&" } }],
        raw: { encrypted: "must not survive" },
        rawFrom: "openai-codex",
        usage: {
          inputTokens: 10,
          outputTokens: 3,
          cachedInputTokens: 2,
          cacheWriteInputTokens: 0,
          reasoningTokens: 1,
        },
      },
      {
        role: "user",
        content: [{ kind: "tool_result", id: "call-1", output: "", isError: false }],
      },
    ],
    blocks: [
      { kind: "user", text: "change it" },
      { kind: "notice", text: "not durable", tone: "info" },
      { kind: "reasoning", text: "inspect", live: true, expanded: true },
      {
        kind: "tool",
        name: "edit_file",
        target: "",
        right: "",
        tone: "ok",
        durationMs: 37,
        body: [{ kind: "add", text: "", newLine: 1, emphasis: { start: 0, length: 1 } }],
      },
    ],
    context: {
      throughNodeId: 1,
      messageCount: 1,
      createdAt: "2026-09-01T10:00:30.000Z",
      summary: "The user requested a safe edit.",
    },
  }, "checkpointed");

  const encoded = encodeNode(
    tree.activeNode as NonNullable<typeof tree.activeNode>,
    7,
    "2026-09-01T10:01:00.000Z",
  );
  assert.doesNotMatch(encoded, /must not survive|rawFrom|"raw"/);
  const decoded = decodeNode(JSON.parse(encoded));

  assert.equal(decoded.sequence, 7);
  assert.equal(decoded.updatedAt, "2026-09-01T10:01:00.000Z");
  assert.deepEqual(decoded.node.messages, tree.activeNode?.messages.map((message) => ({
    role: message.role,
    content: message.content,
    ...(message.usage === undefined ? {} : { usage: message.usage }),
  })));
  assert.deepEqual(decoded.node.blocks, [
    { kind: "user", text: "change it" },
    { kind: "reasoning", text: "inspect" },
    {
      kind: "tool",
      name: "edit_file",
      target: "",
      right: "",
      tone: "ok",
      durationMs: 37,
      body: [{ kind: "add", text: "", newLine: 1, emphasis: { start: 0, length: 1 } }],
    },
  ]);
  assert.deepEqual(decoded.node.context, tree.activeNode?.context);
});

test("schema 1 session nodes remain readable without a context anchor", () => {
  const decoded = decodeNode({
    version: 1,
    sequence: 1,
    updatedAt: "2026-09-01T10:01:00.000Z",
    node: {
      id: 1,
      parentId: 0,
      revision: 1,
      createdAt: "2026-09-01T10:00:00.000Z",
      settlement: "completed",
      identity: { providerId: "ollama", model: "m", effort: "high" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "hello" }], usage: null },
        { role: "assistant", content: [{ kind: "text", text: "hi" }], usage: null },
      ],
      blocks: [],
    },
  });

  assert.equal(decoded.node.context, undefined);
  assert.equal(decoded.node.messages.length, 2);
});

test("schema 2 session nodes remain readable with their context anchor", () => {
  const decoded = decodeNode({
    version: 2,
    sequence: 1,
    updatedAt: "2026-09-01T10:01:00.000Z",
    node: {
      id: 1,
      parentId: 0,
      revision: 1,
      createdAt: "2026-09-01T10:00:00.000Z",
      settlement: "completed",
      identity: { providerId: "ollama", model: "m", effort: "high" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "hello" }], usage: null },
        { role: "assistant", content: [{ kind: "text", text: "hi" }], usage: null },
      ],
      blocks: [],
      context: {
        throughNodeId: 1,
        messageCount: 2,
        createdAt: "2026-09-01T10:00:30.000Z",
        summary: "The user greeted the assistant.",
      },
    },
  });

  assert.equal(decoded.node.context?.summary, "The user greeted the assistant.");
  assert.equal(decoded.node.failure, undefined);
});

test("schema 3 tool records remain readable without a duration", () => {
  const decoded = decodeNode({
    version: 3,
    sequence: 1,
    updatedAt: "2026-09-01T10:01:00.000Z",
    node: {
      id: 1,
      parentId: 0,
      revision: 1,
      createdAt: "2026-09-01T10:00:00.000Z",
      settlement: "completed",
      identity: { providerId: "ollama", model: "m", effort: "high" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "inspect" }], usage: null },
        { role: "assistant", content: [{ kind: "text", text: "done" }], usage: null },
      ],
      blocks: [{
        kind: "tool",
        name: "read_file",
        target: "README.md",
        right: "40 lines",
        tone: "ok",
        body: null,
      }],
      context: null,
      failure: null,
    },
  });

  assert.deepEqual(decoded.node.blocks, [{
    kind: "tool",
    name: "read_file",
    target: "README.md",
    right: "40 lines",
    tone: "ok",
  }]);
});

test("current session tool durations are bounded non-negative integers", () => {
  const tree = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "ollama", model: "m", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "inspect" }] },
      { role: "assistant", content: [{ kind: "text", text: "done" }] },
    ],
    blocks: [{
      kind: "tool",
      name: "read_file",
      target: "README.md",
      right: "40 lines",
      tone: "ok",
      durationMs: 7,
    }],
  }, "completed");
  const encoded = JSON.parse(encodeNode(
    tree.activeNode as NonNullable<typeof tree.activeNode>,
    1,
    "2026-09-01T10:01:00.000Z",
  ));
  encoded.node.blocks[0].durationMs = -1;

  assert.throws(() => decodeNode(encoded), /invalid or unsupported/);
});

test("session metadata and head codecs reject unknown fields", () => {
  const meta = {
    version: 1 as const,
    id: "session-1",
    workspaceRoot: "C:\\work",
    workspaceDigest: "a".repeat(64),
    createdAt: "2026-09-01T10:00:00.000Z",
  };
  const head = {
    version: 1 as const,
    sequence: 2,
    nodeId: 1,
    parentId: 0,
    revision: 1,
    updatedAt: "2026-09-01T10:01:00.000Z",
  };
  assert.deepEqual(decodeMeta(JSON.parse(encodeMeta(meta))), meta);
  assert.deepEqual(decodeHead(JSON.parse(encodeHead(head))), head);
  assert.throws(() => decodeMeta({ ...meta, secret: true }), /invalid or unsupported/);
  assert.throws(() => decodeHead({ ...head, future: true }), /invalid or unsupported/);
  assert.throws(
    () => encodeMeta({ ...meta, workspaceRoot: "😀".repeat(16_384) }),
    /invalid or unsupported/,
  );
});

test("session codec rejects unbounded and malformed tool input", () => {
  const malformed = {
    version: 1,
    sequence: 1,
    updatedAt: "2026-09-01T10:01:00.000Z",
    node: {
      id: 1,
      parentId: 0,
      revision: 1,
      createdAt: "2026-09-01T10:00:00.000Z",
      settlement: "checkpointed",
      identity: { providerId: "ollama", model: "m", effort: "high" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "x" }], usage: null },
        {
          role: "assistant",
          content: [{ kind: "tool_call", id: "c", name: "run_command", input: { value: Infinity } }],
          usage: null,
        },
      ],
      blocks: [],
    },
  };
  assert.throws(() => decodeNode(malformed), /invalid or unsupported/);
});

test("conversation commits enforce the same per-field boundary as the session decoder", () => {
  const atLimit = "a".repeat(SESSION_FILE_LIMITS.text);
  const accepted = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "ollama", model: "m", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: atLimit }] },
      { role: "assistant", content: [{ kind: "text", text: "ok" }] },
    ],
    blocks: [{ kind: "user", text: atLimit }, { kind: "answer", text: "ok" }],
  }, "completed");

  assert.doesNotThrow(() => encodeNode(
    accepted.activeNode as NonNullable<typeof accepted.activeNode>,
    1,
    "2026-09-01T10:01:00.000Z",
  ));
  assert.throws(() => ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "ollama", model: "m", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: `${atLimit}a` }] },
      { role: "assistant", content: [{ kind: "text", text: "ok" }] },
    ],
    blocks: [],
  }, "completed"), /invalid or unsupported/);
});

test("conversation commits enforce the session file's UTF-8 byte boundary", () => {
  const messageText = "😀".repeat(SESSION_FILE_LIMITS.text / 2);
  const blockText = "😀".repeat(523_500);
  const summary = "😀".repeat(16_384);

  assert.throws(() => ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "ollama", model: "m", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: messageText }] },
      { role: "assistant", content: [{ kind: "text", text: messageText }] },
    ],
    blocks: Array.from({ length: 8 }, (_, index) => ({
      kind: index % 2 === 0 ? "user" as const : "answer" as const,
      text: blockText,
    })),
    context: {
      throughNodeId: 1,
      messageCount: 2,
      createdAt: "2026-09-01T10:00:30.000Z",
      summary,
    },
  }, "completed"), /invalid or unsupported/);
});
