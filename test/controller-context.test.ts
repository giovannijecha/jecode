import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { resolveContextPolicy } from "../src/context/capacity.ts";
import { estimateRequestInputTokens } from "../src/context/budget.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { runTurn } from "../src/controller.ts";
import type { Tool } from "../src/tools/index.ts";
import { scripted, echo, options, events, assistantText, texts } from "../dev/test-support/controller.ts";

test("clamps output so the complete request fits a small model window", async () => {
  const provider = scripted([assistantText("all done")]);
  let capacityChecks = 0;
  let sentInputTokens = 0;
  const send = provider.send.bind(provider);
  provider.contextWindow = async () => {
    capacityChecks++;
    return { tokens: 4_096 };
  };
  provider.send = async (request) => {
    sentInputTokens = estimateRequestInputTokens(request);
    return send(request);
  };
  const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "hi" }] }];
  const configured = 64_000;
  const contextPolicy = () => resolveContextPolicy({
    provider,
    model: "fake-1",
    compactionPercent: 85,
  });

  await runTurn(history, options(provider, { maxTokens: configured, contextPolicy }), events());

  const request = provider.seen[0] as SendRequest;
  assert.equal(capacityChecks, 1);
  assert.ok(request.maxTokens < configured);
  assert.ok(sentInputTokens + request.maxTokens <= 4_096);
});

test("rejects an oversized tool envelope before opening a provider request", async () => {
  const provider = scripted([assistantText("unreachable")]);
  const oversized: Tool = {
    ...echo,
    description: "x".repeat(15_000),
  };
  const policy = policyForContextWindow({ tokens: 4_096 }, 85);

  await assert.rejects(
    runTurn(
      [{ role: "user", content: [{ kind: "text", text: "hi" }] }],
      options(provider, {
        tools: [oversized],
        maxTokens: 64_000,
        contextPolicy: () => Promise.resolve(policy),
      }),
      events(),
    ),
    /request input needs approximately .* at least 256 output tokens are required/,
  );

  assert.equal(provider.seen.length, 0);
});

test("refreshes model capacity between provider requests in a turn", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
    },
    assistantText("done"),
  ]);
  let resolutions = 0;

  await runTurn([], options(provider, {
    maxTokens: 64_000,
    contextPolicy: async () => {
      resolutions++;
      return policyForContextWindow({ tokens: resolutions === 1 ? 32_000 : 4_096 }, 85);
    },
  }), events());

  assert.equal(provider.seen.length, 2);
  assert.equal(resolutions, 2);
  assert.ok((provider.seen[1]?.maxTokens ?? Infinity) < 4_096);
});

test("keeps canonical turn history while replacing only the provider context", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
    },
    assistantText("done"),
  ]);
  const history: Message[] = [
    { role: "user", content: [{ kind: "text", text: "full request" }] },
  ];
  const modelHistory: Message[] = [
    { role: "user", content: [{ kind: "text", text: "projected request" }] },
  ];
  const sink = events();
  sink.onCheckpoint = async (_history, settlement) => settlement === "checkpointed"
    ? [{ role: "user", content: [{ kind: "text", text: "compacted checkpoint" }] }]
    : undefined;

  await runTurn(history, options(provider), sink, undefined, modelHistory);

  assert.equal(texts(provider.seen[1]?.messages ?? [])[0], "compacted checkpoint");
  assert.doesNotMatch(texts(provider.seen[1]?.messages ?? []).join("\n"), /full request/);
  assert.deepEqual(texts(history), ["full request", "done"]);
  assert.equal(history.length, 4);
});

test("clips aggregate tool output only on the provider request", async () => {
  const provider = scripted([assistantText("done")]);
  const output = "evidence ".repeat(1_000);
  const history: Message[] = [{
    role: "user",
    content: [{ kind: "tool_result", id: "large", output, isError: false }],
  }];
  const constrained = policyForContextWindow({ tokens: 4_096 }, 85);

  await runTurn(history, options(provider, {
    contextPolicy: () => Promise.resolve(constrained),
  }), events());

  const sent = provider.seen[0]?.messages[0]?.content[0];
  assert.equal(sent?.kind, "tool_result");
  assert.ok(sent?.kind === "tool_result" && sent.output.length < output.length);
  assert.match(sent?.kind === "tool_result" ? sent.output : "", /\[tool output clipped\]/);
  assert.equal(history[0]?.content[0]?.kind === "tool_result"
    ? history[0].content[0].output
    : "", output);
});

test("rebudgets the newest-first fallback from the exact request projection", async () => {
  const provider = scripted([assistantText("done")]);
  const constrained = policyForContextWindow({ tokens: 4_096 }, 85);
  const history: Message[] = [{
    role: "user",
    content: [
      { kind: "tool_result", id: "old", output: "a".repeat(10_000), isError: false },
      {
        kind: "tool_result",
        id: "new",
        output: String.fromCodePoint(0x10ffff).repeat(10_000),
        isError: false,
      },
    ],
  }];

  await runTurn(history, options(provider, {
    maxTokens: 3_600,
    contextPolicy: () => Promise.resolve(constrained),
  }), events());

  const request = provider.seen[0] as SendRequest;
  assert.ok(
    estimateRequestInputTokens(request) + request.maxTokens <= constrained.requestLimitTokens,
  );
});

test("retries one definite context rejection only after the context hook replaces it", async () => {
  const seen: Message[][] = [];
  const requests: SendRequest[] = [];
  let calls = 0;
  const provider: Provider = {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    async send(request) {
      requests.push(request);
      seen.push(structuredClone(request.messages));
      calls++;
      if (calls === 1) {
        throw Object.assign(new Error("request rejected"), {
          status: 400,
          body: '{"error":{"code":"context_length_exceeded"}}',
        });
      }
      return assistantText("recovered");
    },
  };
  const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "hello" }] }];
  const reasons: string[] = [];
  let resolutions = 0;
  const sink = events();
  sink.onContext = async (_history, _context, request) => {
    reasons.push(request.reason);
    if (request.reason !== "overflow" || request.error === undefined) return undefined;
    return [{ role: "user", content: [{ kind: "text", text: "safe summary" }] }];
  };

  await runTurn(history, options(provider, {
    maxTokens: 64_000,
    contextPolicy: async () => {
      resolutions++;
      return policyForContextWindow({ tokens: resolutions === 1 ? 32_000 : 4_096 }, 85);
    },
  }), sink);

  assert.equal(calls, 2);
  assert.equal(resolutions, 2);
  assert.ok((requests[1]?.maxTokens ?? Infinity) < 4_096);
  assert.deepEqual(reasons, ["budget", "overflow"]);
  assert.deepEqual(texts(seen[1] ?? []), ["safe summary"]);
  assert.deepEqual(texts(history), ["hello", "recovered"]);
});
