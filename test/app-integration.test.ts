import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { CONVERSATION_LIMITS, ConversationTree, type TurnNode } from "../src/conversation.ts";
import type { Tool } from "../src/tools/index.ts";
import type { Message, Provider } from "../src/types.ts";
import { runApp } from "../src/tui/app.ts";
import { MAX_PROMPT_CODE_UNITS } from "../src/input-boundary.ts";
import { provider, session } from "../dev/test-support/app.ts";
import {
  virtualScreen,
  waitFor,
  lastFooter,
  waitForIdle,
  waitForExportCompletion,
  deferred,
} from "../dev/test-support/app-harness.ts";

function conversationAtLimit(): ConversationTree {
  const nodes: TurnNode[] = Array.from({ length: CONVERSATION_LIMITS.nodes }, (_, index) => ({
    id: index + 1,
    parentId: index,
    revision: 1,
    createdAt: "2026-09-04T10:00:00.000Z",
    settlement: "completed",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: `question ${index}` }] },
      { role: "assistant", content: [{ kind: "text", text: `answer ${index}` }] },
    ],
    blocks: [],
  }));
  return ConversationTree.restore(nodes, nodes.length);
}

test("a TUI submit reaches the provider and returns to an editable session", async () => {
  const harness = virtualScreen();
  const current = session(provider("Answer from the TUI."));
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("hello\r");
  await waitFor(() => current.conversation.history.length === 2, "completed TUI turn");
  await waitFor(
    () => harness.frames.flat().join("\n").includes("Answer from the TUI."),
    "painted TUI answer",
  );
  assert.equal(current.conversation.history[0]?.role, "user");
  assert.equal(current.conversation.history[1]?.role, "assistant");
  assert.match(harness.frames.flat().join("\n"), /Answer from the TUI\./);

  feed("/exit\r");
  await running;
  assert.equal(harness.left(), true);
});

test("an oversized TUI paste stays in the footer and never reaches the provider", async () => {
  let requests = 0;
  const counted: Provider = {
    ...provider(),
    async send(): Promise<Message> {
      requests++;
      return { role: "assistant", content: [{ kind: "text", text: "unexpected" }] };
    },
  };
  const current = session(counted);
  const harness = virtualScreen(180);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();
  const escape = String.fromCharCode(27);

  feed("keep");
  feed(`${escape}[200~${"x".repeat(MAX_PROMPT_CODE_UNITS + 1)}${escape}[201~\r`);
  await waitFor(
    () => lastFooter(harness).includes("1,048,576 UTF-16 code units"),
    "prompt limit feedback",
  );

  assert.equal(requests, 0);
  assert.deepEqual(current.conversation.history, []);
  assert.match(harness.frames.at(-1)?.join("\n") ?? "", /keep/);

  feed(String.fromCharCode(21));
  feed("/exit\r");
  await running;
  assert.equal(harness.left(), true);
});

test("a full conversation keeps the next prompt and never reaches the provider", async () => {
  let requests = 0;
  const counted: Provider = {
    ...provider(),
    async send(): Promise<Message> {
      requests++;
      return { role: "assistant", content: [{ kind: "text", text: "unexpected" }] };
    },
  };
  const current = session(counted);
  current.conversation = conversationAtLimit();
  const original = current.conversation;
  const harness = virtualScreen(180);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("keep this prompt\r");
  await waitFor(
    () => lastFooter(harness).includes("conversation reached its session limit"),
    "session limit feedback",
  );

  assert.equal(requests, 0);
  assert.equal(current.conversation, original);
  assert.equal(current.conversation.nodes.length, CONVERSATION_LIMITS.nodes);

  feed(String.fromCharCode(12));
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("keep this prompt"),
    "kept session-limit prompt",
  );

  feed(String.fromCharCode(3));
  await running;
  assert.equal(harness.left(), true);
});

test("the footer follows model, tool preparation, execution, and response phases", async () => {
  const connecting = deferred();
  const waiting = deferred();
  const working = deferred();
  const thinking = deferred();
  const preparing = deferred();
  const executing = deferred();
  const responding = deferred();
  let requests = 0;
  const phased: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      requests++;
      if (requests === 1) {
        request.onStatus?.("Connecting");
        await connecting.wait;
        request.onStatus?.("Waiting for model");
        await waiting.wait;
        request.onStatus?.("Working");
        await working.wait;
        request.onStream?.({ kind: "thinking", text: "Inspecting" });
        await thinking.wait;
        request.onStream?.({ kind: "tool", name: "probe" });
        await preparing.wait;
        return {
          role: "assistant",
          content: [{ kind: "tool_call", id: "probe-1", name: "probe", input: {} }],
        };
      }

      request.onStream?.({ kind: "text", text: "Finished." });
      await responding.wait;
      return { role: "assistant", content: [{ kind: "text", text: "Finished." }] };
    },
  };
  const probe: Tool = {
    name: "probe",
    description: "waits while the footer is inspected",
    dangerous: false,
    concurrency: "shared",
    input: { type: "object", properties: {}, required: [] },
    async run() {
      await executing.wait;
      return { output: "done" };
    },
  };
  const current = session(phased);
  current.tools = [probe];
  const harness = virtualScreen(120);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("inspect\r");
  await waitFor(() => lastFooter(harness).includes("Connecting"), "connecting footer phase");
  connecting.release();
  await waitFor(
    () => lastFooter(harness).includes("Waiting for model"),
    "model wait footer phase",
  );
  waiting.release();
  await waitFor(() => lastFooter(harness).includes("Working ·"), "working footer phase");
  working.release();
  await waitFor(() => lastFooter(harness).includes("Thinking ·"), "thinking footer phase");
  thinking.release();
  await waitFor(
    () => lastFooter(harness).includes("Preparing probe ·"),
    "tool preparation footer phase",
  );
  preparing.release();
  await waitFor(
    () => lastFooter(harness).includes("Running probe ·"),
    "tool execution footer phase",
  );
  executing.release();
  await waitFor(() => lastFooter(harness).includes("Responding ·"), "response footer phase");
  responding.release();
  await waitFor(() => current.conversation.history.length === 4, "completed phased turn");
  await waitFor(
    () => harness.frames.flat().join("\n").includes("Finished."),
    "completed answer",
  );

  feed("/exit\r");
  await running;
  assert.equal(harness.left(), true);
});

test("escape cancels a command without leaving interrupted feedback", async () => {
  let signal: AbortSignal | undefined;
  const waiting: Provider = {
    ...provider(),
    efforts: (_model, current) => {
      signal = current;
      return new Promise<readonly string[]>((_resolve, reject) => {
        current?.addEventListener("abort", () => reject(current.reason), { once: true });
      });
    },
  };
  const harness = virtualScreen();
  const running = runApp(session(waiting), process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("/effort\r");
  await waitFor(() => signal !== undefined, "effort discovery");
  feed(String.fromCharCode(27));
  await waitFor(() => signal?.aborted === true, "command cancellation");
  await waitFor(
    () => !(harness.frames.at(-1) ?? []).join("\n").includes("esc to interrupt"),
    "quiet command footer",
  );
  assert.doesNotMatch((harness.frames.at(-1) ?? []).join("\n"), /interrupted/);

  feed("/exit\r");
  await running;
  assert.equal(harness.left(), true);
});

test("/export writes without a picker to the directory where Jecode was launched", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-tui-export-"));
  const harness = virtualScreen();
  const running = runApp(session(provider("Exported answer.")), directory, harness.environment);
  const feed = await harness.input();

  try {
    feed("keep this\r");
    await waitFor(
      () => harness.frames.flat().join("\n").includes("Exported answer."),
      "answer before export",
    );
    await waitForIdle(harness, "completed answer before export");
    const exportStartedAt = harness.frames.length;
    feed("/export\r");
    let name: string | undefined;
    await waitFor(async () => {
      name = (await readdir(directory)).find((entry) => /^jecode-transcript-.*\.md$/.test(entry));
      return name !== undefined;
    }, "transcript export");
    assert.ok(name !== undefined);
    const markdown = await readFile(path.join(directory, name), "utf8");
    assert.match(markdown, /keep this/);
    assert.match(markdown, /Exported answer\./);
    await waitForExportCompletion(harness, exportStartedAt, "export command");
    feed("/exit\r");
    await running;
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("/help stays in the dock and disappears when escape closes it", async () => {
  const harness = virtualScreen();
  const running = runApp(session(), process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("/help\r");
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("keyboard controls"),
    "help dock",
  );
  const openedAt = harness.frames.length;

  feed(String.fromCharCode(27));
  await waitFor(
    () => harness.frames.length > openedAt &&
      !(harness.frames.at(-1) ?? []).join("\n").includes("keyboard controls"),
    "closed help dock",
  );

  feed("/exit\r");
  await running;
  assert.equal(harness.left(), true);
});
