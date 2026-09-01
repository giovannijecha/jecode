import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { runBatch } from "../src/batch.ts";
import { ConversationTree } from "../src/conversation.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import type { Session } from "../src/session.ts";
import { start } from "../src/start.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import type { Painter } from "../src/tui/frame.ts";
import { runApp } from "../src/tui/app.ts";
import type { AppScreen } from "../src/tui/app.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

function provider(reply = "Hello from fake."): Provider {
  return {
    id: "fake",
    defaultModel: "fake-1",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-1"]),
    async send(request: SendRequest): Promise<Message> {
      request.onStream?.({ kind: "text", text: reply });
      return {
        role: "assistant",
        content: [{ kind: "text", text: reply }],
        usage: {
          inputTokens: 7,
          outputTokens: 4,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
}

function session(from = provider()): Session {
  return {
    config: {
      providerId: from.id,
      model: from.defaultModel,
      reducedMotion: true,
      effort: "high",
      maxTokens: 4096,
      maxSteps: 8,
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider: from,
    model: from.defaultModel,
    palette: STEEL,
    tools: [],
    system: "be useful",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}

async function* input(...lines: string[]): AsyncIterable<string> {
  for (const line of lines) yield line;
}

function messageText(message: Message | undefined): string {
  return message?.content.find((block) => block.kind === "text")?.text ?? "";
}

test("batch mode carries input through the controller, renderer, commands, and exit", async () => {
  const output: string[] = [];
  const current = session();

  await runBatch(current, {
    lines: input("hello", "/help", "/exit", "ignored"),
    width: 60,
    write: (text) => output.push(text),
  });

  const shown = output.join("");
  assert.match(shown, /> hello/);
  assert.match(shown, /Hello from fake\./);
  assert.match(shown, /interactive help needs the TUI/);
  assert.doesNotMatch(shown, /ignored/);
  assert.equal(current.conversation.history.length, 2);
});

test("batch mode compacts model context while retaining the complete conversation", async () => {
  const seen: { system: string; messages: Message[] }[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      seen.push({ system: request.system, messages: structuredClone(request.messages) });
      const summary = request.system.includes("durable working memory");
      return {
        role: "assistant",
        content: [{ kind: "text", text: summary ? "Earlier work is summarized." : "Final answer." }],
        usage: {
          inputTokens: summary ? 30_000 : 100,
          outputTokens: 10,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const current = session(compacting);
  const old = "old context ".repeat(10_000);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: old }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.usage.lastInputTokens = 65_000;

  await runBatch(current, { lines: input("next request"), write: () => {} });

  assert.equal(seen.length, 2);
  assert.match(seen[0]?.system ?? "", /durable working memory/);
  assert.match(messageText(seen[1]?.messages[0]), /Earlier conversation summary/);
  assert.doesNotMatch(JSON.stringify(seen[1]?.messages), /old context/);
  assert.match(JSON.stringify(current.conversation.history), /old context/);
  assert.equal(current.conversation.activeNode?.context?.throughNodeId, 2);
  assert.equal(current.conversation.activeNode?.context?.messageCount, 0);
  assert.equal(current.usage.requests, 2);
  assert.equal(current.usage.lastInputTokens, 100);
});

test("batch mode propagates provider failures outside the transcript", async () => {
  const failed: Provider = {
    ...provider(),
    async send(): Promise<Message> {
      throw new Error("fixture provider failed");
    },
  };
  const output: string[] = [];

  await assert.rejects(
    runBatch(session(failed), {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /fixture provider failed/,
  );

  const shown = output.join("");
  assert.match(shown, /> hello/);
  assert.doesNotMatch(shown, /fixture provider failed/);
});

test("batch mode discards an incomplete streamed answer when the provider fails", async () => {
  const failed: Provider = {
    ...provider(),
    async send(request: SendRequest): Promise<Message> {
      request.onStream?.({ kind: "text", text: "partial answer" });
      throw new Error("stream failed");
    },
  };
  const output: string[] = [];

  await assert.rejects(
    runBatch(session(failed), {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /stream failed/,
  );

  assert.doesNotMatch(output.join(""), /partial answer|stream failed/);
});

test("batch mode propagates controller step exhaustion", async () => {
  let sends = 0;
  const looping: Provider = {
    ...provider(),
    async send(): Promise<Message> {
      sends++;
      return {
        role: "assistant",
        content: [{ kind: "tool_call", id: String(sends), name: "missing", input: {} }],
      };
    },
  };
  const current = session(looping);
  current.config.maxSteps = 2;
  const output: string[] = [];

  await assert.rejects(
    runBatch(current, {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /gave up after 2 steps/,
  );

  assert.equal(sends, 2);
  assert.doesNotMatch(output.join(""), /gave up after/);
});

test("the bootstrap selects the interactive surface and builds one complete session", async () => {
  let opened: Session | undefined;
  let transcriptRoot: string | undefined;
  let batchCalled = false;
  const root = path.resolve("test-fixture-root");

  await start(
    [
      "--provider", "anthropic",
      "--model", "fixture-model",
      "--effort", "max",
      "--max-tokens", "1024",
      "--max-steps", "3",
      "--root", root,
      "--ephemeral",
    ],
    {
      applicationRoot: process.cwd(),
      transcriptRoot: root,
      interactive: () => true,
      runInteractive: async (current, destination) => {
        opened = current;
        transcriptRoot = destination;
      },
      runNonInteractive: async () => {
        batchCalled = true;
      },
    },
  );

  assert.equal(batchCalled, false);
  assert.equal(opened?.provider.id, "anthropic");
  assert.equal(opened?.model, "fixture-model");
  assert.equal(opened?.config.root, root);
  assert.equal(opened?.config.effort, "max");
  assert.equal(transcriptRoot, root);
  assert.ok((opened?.tools.length ?? 0) > 0);
  assert.match(opened?.system ?? "", /Workspace root:/);
});

test("resume --latest restores the newest durable conversation for this workspace", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-start-resume-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  let opened: Session | undefined;
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const conversation = ConversationTree.empty().commit({
      parentId: 0,
      createdAt: "2026-09-01T10:00:00.000Z",
      identity: { providerId: "anthropic", model: "claude-resumed", effort: "medium" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "remember me" }] },
        {
          role: "assistant",
          content: [{ kind: "text", text: "remembered" }],
          usage: {
            inputTokens: 12,
            outputTokens: 4,
            cachedInputTokens: 0,
            cacheWriteInputTokens: 0,
            reasoningTokens: 1,
          },
        },
      ],
      blocks: [
        { kind: "user", text: "remember me" },
        { kind: "answer", text: "remembered" },
      ],
      context: {
        throughNodeId: 1,
        messageCount: 2,
        createdAt: "2026-09-01T10:00:30.000Z",
        summary: "The user asked to be remembered and the request was acknowledged.",
      },
    }, "completed");
    const published = await store.publish(conversation);

    await start(["resume", "--latest", "--root", workspace], {
      applicationRoot: process.cwd(),
      sessionsRoot: sessions,
      interactive: () => true,
      runInteractive: async (current) => {
        opened = current;
      },
    });

    assert.equal(opened?.model, "claude-resumed");
    assert.equal(opened?.config.effort, "medium");
    assert.equal(opened?.conversation.history[0]?.content[0]?.kind, "text");
    assert.match(messageText(opened?.conversation.contextHistory[0]), /Earlier conversation summary/);
    assert.equal(opened?.conversation.contextHistory.length, 1);
    assert.equal(opened?.usage.requests, 1);
    assert.equal(opened?.persistence?.sessionId, published.meta.id);
    assert.equal((await store.list())[0]?.active, false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a rejected resume identity leaves the current runtime selection intact", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-start-resume-invalid-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const conversation = ConversationTree.empty().commit({
      parentId: 0,
      createdAt: "2026-09-01T10:00:00.000Z",
      identity: { providerId: "removed-provider", model: "old-model", effort: "low" },
      messages: [
        { role: "user", content: [{ kind: "text", text: "old question" }] },
        { role: "assistant", content: [{ kind: "text", text: "old answer" }] },
      ],
      blocks: [
        { kind: "user", text: "old question" },
        { kind: "answer", text: "old answer" },
      ],
    }, "completed");
    const published = await store.publish(conversation);

    await start([
      "resume",
      "--root", workspace,
      "--provider", "anthropic",
      "--model", "baseline-model",
      "--effort", "high",
    ], {
      applicationRoot: process.cwd(),
      sessionsRoot: sessions,
      interactive: () => true,
      runInteractive: async (current) => {
        const resume = current.resume;
        assert.ok(resume !== undefined);
        await assert.rejects(resume.open(published.meta.id), /unknown provider/);
        assert.equal(current.config.providerId, "anthropic");
        assert.equal(current.config.model, "baseline-model");
        assert.equal(current.config.effort, "high");
        assert.equal(current.provider.id, "anthropic");
        assert.equal(current.model, "baseline-model");
        assert.equal(current.resume, resume);
      },
    });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("plain resume starts in the shared searchable picker", async () => {
  const current = session();
  const restored = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "saved question" }] },
      { role: "assistant", content: [{ kind: "text", text: "saved answer" }] },
    ],
    blocks: [
      { kind: "user", text: "saved question" },
      { kind: "answer", text: "saved answer" },
    ],
  }, "completed");
  current.resume = {
    candidates: [{
      id: "saved-1",
      createdAt: "2026-09-01T10:00:00.000Z",
      updatedAt: "2026-09-01T10:01:00.000Z",
      turns: 1,
      preview: "saved question",
      active: false,
    }],
    open: async () => {
      current.conversation = restored;
      current.resume = undefined;
    },
  };
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("saved question"),
    "resume picker",
  );
  feed("\r");
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("saved answer"),
    "restored transcript",
  );
  feed("/exit\r");
  await running;
});

test("the TUI owns and restores a screen around a real /exit interaction", async () => {
  let feed: ((chunk: string) => void) | undefined;
  let entered = false;
  let left = false;
  let inputStopped = false;
  let resizeStopped = false;
  const frames: string[][] = [];

  const screen: AppScreen = {
    size: () => ({ rows: 18, cols: 70 }),
    enter: () => {
      entered = true;
    },
    leave: () => {
      left = true;
    },
    setReducedMotion: () => {},
    onResize: () => () => {
      resizeStopped = true;
    },
    onInput: (handler) => {
      feed = handler;
      return () => {
        inputStopped = true;
      };
    },
  };
  const paint: Painter = {
    paint: (rows) => frames.push([...rows]),
    invalidate: () => {},
  };

  const running = runApp(session(), process.cwd(), { screen, paint });
  while (feed === undefined) await new Promise<void>((resolve) => setImmediate(resolve));
  feed("/exit\r");
  await running;

  assert.equal(entered, true);
  assert.equal(left, true);
  assert.equal(inputStopped, true);
  assert.equal(resizeStopped, true);
  assert.ok(frames.length > 0);
  assert.ok(frames.every((frame) => frame.length === 18));
});

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

test("the footer reports compaction without adding it to the transcript", async () => {
  let started = (): void => {};
  const entered = new Promise<void>((resolve) => {
    started = resolve;
  });
  let release = (): void => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      if (request.system.includes("durable working memory")) {
        started();
        await gate;
        return {
          role: "assistant",
          content: [{ kind: "text", text: "Persisted summary." }],
          usage: {
            inputTokens: 30_000,
            outputTokens: 10,
            cachedInputTokens: 0,
            cacheWriteInputTokens: 0,
            reasoningTokens: 0,
          },
        };
      }
      request.onStream?.({ kind: "text", text: "Answer after compaction." });
      return {
        role: "assistant",
        content: [{ kind: "text", text: "Answer after compaction." }],
        usage: {
          inputTokens: 100,
          outputTokens: 5,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const current = session(compacting);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-01T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(10_000) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.usage.lastInputTokens = 65_000;
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("next request\r");
  await entered;
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("Compacting"),
    "compaction footer state",
  );
  release();
  await waitFor(() => current.conversation.history.length === 4, "compacted TUI turn");
  assert.doesNotMatch(JSON.stringify(current.conversation.transcript), /Persisted summary/);

  feed("/exit\r");
  await running;
});

test("the TUI durably checkpoints turns and /new starts a separate session", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-tui-sessions-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const current = session(provider("Durable answer."));
    current.config.root = workspace;
    current.persistence = SessionPersistence.fresh(store);
    const harness = virtualScreen();
    const running = runApp(current, workspace, harness.environment);
    const feed = await harness.input();

    feed("first\r");
    await waitFor(() => current.conversation.history.length === 2, "first durable turn");
    await waitFor(async () => (await store.list()).length === 1, "first session file");
    const firstId = current.persistence.sessionId;
    assert.ok(firstId !== null);

    feed("/new\r");
    await waitFor(() => current.conversation.history.length === 0, "new session reset");
    assert.equal(current.persistence.sessionId, null);
    assert.equal((await store.list())[0]?.active, false);

    feed("second\r");
    await waitFor(() => current.conversation.history.length === 2, "second durable turn");
    await waitFor(async () => (await store.list()).length === 2, "second session file");
    assert.notEqual(current.persistence.sessionId, firstId);

    feed("/exit\r");
    await running;
    assert.ok((await store.list()).every((entry) => !entry.active));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("only the latest input chunk owns the escape grace timer", async () => {
  const harness = virtualScreen();
  const current = session();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  try {
    feed("draft");
    await delay(15);
    feed(String.fromCharCode(27));
    await delay(15);
    feed("[A");
    await delay(35);
    feed("\r");

    await waitFor(() => current.conversation.history.length === 2, "turn after split cursor sequence");
    const prompt = current.conversation.history[0]?.content[0];
    assert.equal(prompt?.kind === "text" ? prompt.text : undefined, "draft");
  } finally {
    feed("/exit\r");
    await running;
  }
});

test("escape cancels the active provider request before the TUI closes", async () => {
  let signal: AbortSignal | undefined;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      signal = request.signal;
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const harness = virtualScreen();
  const running = runApp(session(waiting), process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("wait\r");
  await waitFor(() => signal !== undefined, "provider request");
  feed(String.fromCharCode(27));
  await waitFor(() => signal?.aborted === true, "provider cancellation");
  await waitFor(() => harness.frames.flat().join("\n").includes("[interrupted]"), "interrupted transcript");

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

test("the packaged jecode executable reaches batch help without a developer script", async () => {
  const result = await runExecutable(["--help"]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /Usage:\s+jecode \[options\]/);
  assert.match(result.stdout, /--provider/);
  assert.match(result.stdout, /type \/ to discover/);
  assert.equal(result.stderr, "");
});

test("the packaged batch executable reports terminal failures on stderr", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-batch-home-"));
  const environment: NodeJS.ProcessEnv = {
    ...process.env,
    HOME: directory,
    USERPROFILE: directory,
    APPDATA: directory,
    LOCALAPPDATA: directory,
    JECODE_HOME: directory,
    XDG_CONFIG_HOME: directory,
  };
  for (const name of Object.keys(environment)) {
    if (name.toUpperCase() === "ANTHROPIC_API_KEY") delete environment[name];
  }

  try {
    const result = await runNode(
      path.resolve("bin/jecode.js"),
      ["--provider", "anthropic", "--model", "fixture-model"],
      { input: "hello\n", environment },
    );

    assert.equal(result.code, 1);
    assert.match(result.stdout, /> hello/);
    assert.doesNotMatch(result.stdout, /ANTHROPIC_API_KEY is not set/);
    assert.match(result.stderr, /^jecode: ANTHROPIC_API_KEY is not set/m);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("the packaged executable reports its manifest version", async () => {
  const manifest = JSON.parse(await readFile("package.json", "utf8")) as { version?: unknown };
  const result = await runExecutable(["--version"]);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout.trim(), manifest.version);
});

test("the launcher rejects the unverified Node 23 type-stripping gap", async () => {
  const result = await runLauncherAsVersion("23.5.0");
  assert.equal(result.code, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /requires Node\.js 22\.18\+ \(22\.x\) or Node\.js 24\+/);
  assert.match(result.stderr, /current: 23\.5\.0/);
});

test("a source-only executable explains how to obtain its missing runtime", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-missing-runtime-"));
  const bin = path.join(directory, "bin");

  try {
    await mkdir(bin);
    await copyFile("package.json", path.join(directory, "package.json"));
    await copyFile("bin/jecode.js", path.join(bin, "jecode.js"));

    const result = await runNode(path.join(bin, "jecode.js"), []);
    assert.equal(result.code, 1);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /install @giovannijecha\/jecode from npm/);
    assert.match(result.stderr, /npm run build:release/);
    assert.doesNotMatch(result.stderr, /ERR_MODULE_NOT_FOUND/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

function runExecutable(args: string[]): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return runNode(path.resolve("bin/jecode.js"), args);
}

function runLauncherAsVersion(
  version: string,
): Promise<{ code: number | null; stdout: string; stderr: string }> {
  const launcher = new URL("../bin/jecode.js", import.meta.url).href;
  const source = [
    `Object.defineProperty(process.versions, "node", { value: ${JSON.stringify(version)} });`,
    `await import(${JSON.stringify(launcher)});`,
  ].join("\n");
  return runNode("--input-type=module", ["--eval", source]);
}

function runNode(
  executable: string,
  args: string[],
  options: { input?: string; environment?: NodeJS.ProcessEnv } = {},
): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const spawnOptions = { cwd: process.cwd(), env: options.environment };
    const child =
      options.input === undefined
        ? spawn(process.execPath, [executable, ...args], {
            ...spawnOptions,
            stdio: ["ignore", "pipe", "pipe"],
          })
        : spawn(process.execPath, [executable, ...args], {
            ...spawnOptions,
            stdio: ["pipe", "pipe", "pipe"],
          });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    if (options.input !== undefined) child.stdin?.end(options.input);
    child.once("error", reject);
    child.once("close", (code) => resolve({ code, stdout, stderr }));
  });
}

function virtualScreen(): {
  environment: { screen: AppScreen; paint: Painter };
  frames: string[][];
  input(): Promise<(chunk: string) => void>;
  left(): boolean;
} {
  let feed: ((chunk: string) => void) | undefined;
  let left = false;
  const frames: string[][] = [];
  const screen: AppScreen = {
    size: () => ({ rows: 18, cols: 70 }),
    enter: () => {},
    leave: () => {
      left = true;
    },
    setReducedMotion: () => {},
    onResize: () => () => {},
    onInput: (handler) => {
      feed = handler;
      return () => {};
    },
  };
  const paint: Painter = {
    paint: (rows) => frames.push([...rows]),
    invalidate: () => {},
  };
  return {
    environment: { screen, paint },
    frames,
    async input() {
      await waitFor(() => feed !== undefined, "TUI input handler");
      return feed as (chunk: string) => void;
    },
    left: () => left,
  };
}

async function waitFor(
  condition: () => boolean | Promise<boolean>,
  label: string,
  timeoutMs = 2_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!(await condition())) {
    if (Date.now() >= deadline) throw new Error(`timed out waiting for ${label}`);
    await new Promise<void>((resolve) => setTimeout(resolve, 5));
  }
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
