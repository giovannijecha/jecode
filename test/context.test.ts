import { test } from "node:test";
import assert from "node:assert/strict";
import { resolveContextPolicy } from "../src/context/capacity.ts";
import { compactContext } from "../src/context/compactor.ts";
import type { ContextPolicy } from "../src/context/policy.ts";
import {
  estimateTokens,
  isContextOverflow,
  planCompaction,
  policyForContextWindow,
  shouldResolveContextPolicy,
} from "../src/context/policy.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";

const policy: ContextPolicy = {
  triggerTokens: 1_000,
  targetTokens: 500,
  recentTokens: 100,
  minimumPrefixTokens: 100,
  summaryMaxTokens: 128,
};

test("derives compaction budgets from each model window and the saved percentage", () => {
  assert.deepEqual(policyForContextWindow({ tokens: 32_000 }, 85), {
    triggerTokens: 27_200,
    targetTokens: 8_000,
    recentTokens: 4_000,
    minimumPrefixTokens: 1_600,
    summaryMaxTokens: 1_024,
  });
  assert.deepEqual(policyForContextWindow({
    tokens: 997_500,
    compactAtTokens: 945_000,
  }, 95), {
    triggerTokens: 945_000,
    targetTokens: 249_375,
    recentTokens: 124_687,
    minimumPrefixTokens: 49_875,
    summaryMaxTokens: 4_096,
  });
  assert.equal(policyForContextWindow(undefined, 85).triggerTokens, 170_000);
});

test("defers capacity discovery until context pressure makes it useful", () => {
  assert.equal(shouldResolveContextPolicy([user("small")], 0), false);
  assert.equal(shouldResolveContextPolicy([user("small")], 16_000), true);
  assert.equal(shouldResolveContextPolicy([], 0, true), true);
});

test("context metadata failure falls back without blocking the turn", async () => {
  const provider: Provider = {
    ...summarizer([], assistant("unused")),
    contextWindow: () => Promise.reject(new Error("catalog unavailable")),
  };

  const resolved = await resolveContextPolicy({
    provider,
    model: "unknown",
    compactionPercent: 85,
  });

  assert.equal(resolved.triggerTokens, 170_000);
});

test("plans a bounded prefix without separating a recent tool call from its result", () => {
  const turn: Message[] = [
    user("inspect"),
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "call-1", name: "read_file", input: { path: "a.ts" } }],
    },
    {
      role: "user",
      content: [{ kind: "tool_result", id: "call-1", output: "done", isError: false }],
    },
  ];
  const context = [user("x".repeat(10_000)), assistant("old"), ...turn];
  const plan = planCompaction(context, turn, 0, 2_000, false, policy);

  assert.equal(plan?.messageCount, 1);
  assert.deepEqual(plan?.tail, turn.slice(1));
  assert.ok(estimateTokens(plan?.prefix ?? []) >= policy.minimumPrefixTokens);
});

test("compacts normalized history and keeps the recent exact tail", async () => {
  const seen: SendRequest[] = [];
  const provider = summarizer(seen, assistant("goal, decisions, files, and next step"));
  const turn = [user("current request"), assistant("current answer")];
  let began = 0;
  let ended = 0;

  const result = await compactContext({
    provider,
    model: "fake-1",
    effort: "high",
    context: [
      user("x".repeat(10_000)),
      {
        ...assistant("old answer"),
        raw: { opaque: true },
        rawFrom: "fake",
      },
      ...turn,
    ],
    turn,
    nodeId: 7,
    coveredMessages: 0,
    lastInputTokens: 2_000,
    policy,
    onBegin: () => began++,
    onEnd: () => ended++,
  });

  assert.equal(began, 1);
  assert.equal(ended, 1);
  assert.equal(seen.length, 1);
  assert.deepEqual(seen[0]?.tools, []);
  assert.match(seen[0]?.system ?? "", /untrusted historical data/);
  assert.equal("raw" in (seen[0]?.messages[1] ?? {}), false);
  assert.equal(result?.anchor.throughNodeId, 7);
  assert.equal(result?.anchor.messageCount, 0);
  assert.match(text(result?.messages[0]), /Earlier conversation summary/);
  assert.deepEqual(result?.messages.slice(1), turn);
  assert.equal(result?.usage?.inputTokens, 20);
});

test("a failed optional summary leaves the original context available", async () => {
  const provider = summarizer([], new Error("summary failed"));
  let ended = 0;
  const result = await compactContext({
    provider,
    model: "fake-1",
    effort: "high",
    context: [user("x".repeat(10_000)), assistant("old"), user("new")],
    turn: [user("new")],
    nodeId: 2,
    coveredMessages: 0,
    lastInputTokens: 2_000,
    policy,
    onEnd: () => ended++,
  });

  assert.equal(result, undefined);
  assert.equal(ended, 1);
});

test("recognizes only definite provider context rejections", () => {
  const overflow = Object.assign(new Error("request rejected"), {
    status: 400,
    body: '{"error":{"code":"context_length_exceeded"}}',
  });
  assert.equal(isContextOverflow(overflow), true);
  assert.equal(isContextOverflow(Object.assign(new Error("context window"), { status: 500 })), false);
  assert.equal(isContextOverflow(new Error("network error calling provider")), false);
});

test("refuses a projection whose recent tail no longer matches canonical history", () => {
  const turn = [user("current")];
  const plan = planCompaction(
    [user("x".repeat(10_000)), user("different")],
    turn,
    0,
    2_000,
    false,
    policy,
  );
  assert.equal(plan, undefined);
});

function summarizer(seen: SendRequest[], outcome: Message | Error): Provider {
  return {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    async send(request) {
      seen.push(request);
      if (outcome instanceof Error) throw outcome;
      return {
        ...outcome,
        usage: {
          inputTokens: 20,
          outputTokens: 5,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
}

function user(value: string): Message {
  return { role: "user", content: [{ kind: "text", text: value }] };
}

function assistant(value: string): Message {
  return { role: "assistant", content: [{ kind: "text", text: value }] };
}

function text(message: Message | undefined): string {
  return message?.content.find((block) => block.kind === "text")?.text ?? "";
}
