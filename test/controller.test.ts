import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message } from "../src/types.ts";
import { runTurn } from "../src/controller.ts";
import { runCommand } from "../src/tools/shell.ts";
import { scripted, options, events, assistantText } from "../dev/test-support/controller.ts";

function restoreEnvironment(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}

test("returns as soon as the model stops asking for tools", async () => {
  const provider = scripted([assistantText("all done")]);
  const history: Message[] = [{ role: "user", content: [{ kind: "text", text: "hi" }] }];
  const sink = events();

  await runTurn(history, options(provider), sink);

  assert.deepEqual(sink.texts, ["all done"]);
  assert.equal(provider.seen.length, 1);
  assert.equal(history.length, 2);
});

test("rejects an empty provider response instead of ending the turn silently", async () => {
  const provider = scripted([{ role: "assistant", content: [] }]);

  await assert.rejects(
    runTurn([], options(provider), events()),
    /completed without an answer or tool call/,
  );
});

test("returns every result of one response in a single message", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [
        { kind: "tool_call", id: "a", name: "echo", input: { text: "one" } },
        { kind: "tool_call", id: "b", name: "echo", input: { text: "two" } },
      ],
    },
    assistantText("done"),
  ]);
  const history: Message[] = [];

  await runTurn(history, options(provider), events());

  const results = history[1];
  assert.equal(results?.role, "user");
  assert.equal(results?.content.length, 2);
  assert.deepEqual(
    results?.content.map((block) => (block.kind === "tool_result" ? block.output : "")),
    ["one", "two"],
  );
});

test("awaits the durable tool checkpoint before asking the provider again", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
    },
    assistantText("done"),
  ]);
  let release = (): void => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let entered = (): void => {};
  const checkpointStarted = new Promise<void>((resolve) => {
    entered = resolve;
  });
  const settlements: string[] = [];
  const sink = events();
  sink.onCheckpoint = async (_history, settlement) => {
    settlements.push(settlement);
    if (settlement === "checkpointed") {
      entered();
      await gate;
    }
  };

  const running = runTurn([], options(provider), sink);
  await checkpointStarted;
  assert.equal(provider.seen.length, 1);
  release();
  await running;

  assert.equal(provider.seen.length, 2);
  assert.deepEqual(settlements, ["checkpointed", "completed"]);
});

test("a throwing tool is reported to the model rather than crashing the turn", async () => {
  const provider = scripted([
    { role: "assistant", content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "boom" } }] },
    assistantText("recovered"),
  ]);
  const history: Message[] = [];

  await runTurn(history, options(provider), events());

  const result = history[1]?.content[0];
  assert.equal(result?.kind === "tool_result" && result.isError, true);
  assert.match(result?.kind === "tool_result" ? result.output : "", /exploded/);
});

test("an unknown tool is reported to the model", async () => {
  const provider = scripted([
    { role: "assistant", content: [{ kind: "tool_call", id: "a", name: "nope", input: {} }] },
    assistantText("ok"),
  ]);
  const history: Message[] = [];

  await runTurn(history, options(provider), events());

  const result = history[1]?.content[0];
  assert.match(result?.kind === "tool_result" ? result.output : "", /no such tool/);
});

test("an explicit internal request budget stops a deterministic run", async () => {
  const looping = Array.from({ length: 5 }, (): Message => ({
    role: "assistant",
    content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "again" } }],
  }));
  const provider = scripted(looping);

  await assert.rejects(
    runTurn([], options(provider, { maxModelRequests: 3 }), events()),
    /stopped after 3 model requests \(request budget reached\)/,
  );
});

test("runs beyond the former default ceiling when no budget is configured", async () => {
  const replies: Message[] = Array.from({ length: 41 }, (_, index) => ({
    role: "assistant",
    content: [{
      kind: "tool_call",
      id: String(index),
      name: "echo",
      input: { text: "again" },
    }],
  }));
  replies.push(assistantText("done"));
  const provider = scripted(replies);

  await runTurn([], options(provider), events());

  assert.equal(provider.seen.length, 42);
});

test("reports provider usage and tool lifecycle", async () => {
  const provider = scripted([
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "a", name: "echo", input: { text: "one" } }],
      usage: {
        inputTokens: 10,
        outputTokens: 2,
        cachedInputTokens: 4,
        cacheWriteInputTokens: 0,
        reasoningTokens: 1,
      },
    },
    assistantText("done"),
  ]);
  const progress: string[] = [];
  const sink = events();
  sink.onUsage = (usage) => progress.push(`usage:${usage.inputTokens}`);
  sink.onToolPreparing = (call, current, total) =>
    progress.push(`preparing:${call.id}:${current}/${total}`);
  sink.onToolStart = (call, current, total) =>
    progress.push(`running:${call.id}:${current}/${total}`);

  await runTurn([], options(provider), sink);

  assert.deepEqual(progress, [
    "usage:10",
    "preparing:a:1/1",
    "running:a:1/1",
  ]);
});

test("a shell credential cannot reach the provider follow-up or display events", async (context) => {
  const secret = "fixture-controller-credential-4902";
  const keyBefore = process.env["OPENAI_API_KEY"];
  const visibleBefore = process.env["JECODE_REVIEW_VISIBLE"];
  process.env["OPENAI_API_KEY"] = secret;
  process.env["JECODE_REVIEW_VISIBLE"] = secret;
  context.after(() => {
    restoreEnvironment("OPENAI_API_KEY", keyBefore);
    restoreEnvironment("JECODE_REVIEW_VISIBLE", visibleBefore);
  });

  const provider = scripted([
    {
      role: "assistant",
      content: [{
        kind: "tool_call",
        id: "shell",
        name: "run_command",
        input: { command: "node -e \"process.stdout.write(process.env.JECODE_REVIEW_VISIBLE ?? '')\"" },
      }],
    },
    assistantText("done"),
  ]);
  const history: Message[] = [];
  const shown: string[] = [];
  const live: string[] = [];
  const sink = events();
  sink.onToolResult = (_call, result) => shown.push(result.output);
  sink.onToolOutput = (_call, output) => live.push(output);

  await runTurn(history, options(provider, { tools: [runCommand] }), sink);

  const result = history[1]?.content[0];
  assert.equal(result?.kind, "tool_result");
  const sent = result?.kind === "tool_result" ? result.output : "";
  assert.match(sent, /\[credential redacted\]/);
  assert.doesNotMatch(sent, /fixture-controller-credential-4902/);
  assert.deepEqual(shown, [sent]);
  assert.ok(live.length > 0);
  assert.ok(live.every((output) => !output.includes(secret)));
  assert.ok(live.some((output) => output.includes("[credential redacted]")));
});
