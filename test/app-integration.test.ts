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
import type { Tool } from "../src/tools/index.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import type { Painter } from "../src/tui/frame.ts";
import { runApp } from "../src/tui/app.ts";
import type { AppScreen } from "../src/tui/app.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";
import { MAX_PROMPT_CODE_UNITS } from "../src/input-boundary.ts";

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

test("batch mode rejects an oversized line before echo, history, or provider use", async () => {
  let requests = 0;
  const counted: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      requests++;
      return provider().send(request);
    },
  };
  const current = session(counted);
  const output: string[] = [];

  await assert.rejects(
    runBatch(current, {
      lines: input("x".repeat(MAX_PROMPT_CODE_UNITS + 1)),
      write: (text) => output.push(text),
    }),
    /Prompt cannot exceed 1,048,576 UTF-16 code units/,
  );

  assert.equal(requests, 0);
  assert.equal(output.join(""), "");
  assert.deepEqual(current.conversation.history, []);
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

test("missing provider usage replaces stale context pressure with the sent estimate", async () => {
  const requests: string[] = [];
  const withoutUsage: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 3_891 }),
    async send(request): Promise<Message> {
      const summary = request.system.includes("durable working memory");
      requests.push(summary ? "summary" : "normal");
      return {
        role: "assistant",
        content: [{ kind: "text", text: summary ? "Earlier work." : "Answer." }],
      };
    },
  };
  const current = session(withoutUsage);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old context ".repeat(350) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  current.usage.lastInputTokens = 3_500;

  await runBatch(current, { lines: input("first", "second"), write: () => {} });

  assert.deepEqual(requests, ["summary", "normal", "normal"]);
  assert.ok(current.usage.lastInputTokens < 3_500);
});

test("batch /compact updates the next provider projection without transcript noise", async () => {
  const seen: SendRequest[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      seen.push(request);
      const summary = request.system.includes("durable working memory");
      return {
        role: "assistant",
        content: [{ kind: "text", text: summary ? "Manual batch summary." : "After compact." }],
        usage: {
          inputTokens: summary ? 10_000 : 80,
          outputTokens: 8,
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
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old batch context ".repeat(2_000) }] },
      { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
    ],
    blocks: [],
  }, "completed");
  const output: string[] = [];

  await runBatch(current, {
    lines: input("/compact", "next request", "/exit"),
    write: (text) => output.push(text),
  });

  assert.equal(seen.length, 2);
  assert.match(seen[0]?.system ?? "", /durable working memory/);
  assert.match(messageText(seen[1]?.messages[0]), /Earlier conversation summary/);
  assert.doesNotMatch(JSON.stringify(seen[1]?.messages), /old batch context/);
  assert.match(output.join(""), /context compacted/);
  assert.doesNotMatch(JSON.stringify(current.conversation.transcript), /Manual batch summary/);
});

test("batch mode propagates provider failures outside the transcript", async () => {
  const failed: Provider = {
    ...provider(),
    async send(): Promise<Message> {
      throw Object.assign(new Error("fixture provider failed"), {
        body: '{"error":{"message":"requested model is unavailable"}}',
      });
    },
  };
  const output: string[] = [];

  await assert.rejects(
    runBatch(session(failed), {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /fixture provider failed · requested model is unavailable/,
  );

  const shown = output.join("");
  assert.match(shown, /> hello/);
  assert.doesNotMatch(shown, /fixture provider failed|requested model is unavailable/);
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

test("batch mode propagates an explicit model-request budget", async () => {
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
  current.config.maxModelRequests = 2;
  const output: string[] = [];

  await assert.rejects(
    runBatch(current, {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /stopped after 2 model requests/,
  );

  assert.equal(sends, 2);
  assert.doesNotMatch(output.join(""), /stopped after/);
});

test("batch process cancellation reaches the active provider request", async () => {
  let providerSignal: AbortSignal | undefined;
  const waiting: Provider = {
    ...provider(),
    send(request): Promise<Message> {
      providerSignal = request.signal;
      return new Promise((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), {
          once: true,
        });
      });
    },
  };
  const shutdown = new AbortController();
  const running = runBatch(session(waiting), {
    lines: input("wait"),
    signal: shutdown.signal,
    write: () => {},
  });

  await waitFor(() => providerSignal !== undefined, "batch provider request");
  const reason = new Error("received SIGTERM");
  shutdown.abort(reason);

  await assert.rejects(running, (error) => error === reason);
  assert.equal(providerSignal?.aborted, true);
});

test("batch process cancellation interrupts an idle input wait", async () => {
  let waiting = false;
  const lines: AsyncIterable<string> = {
    [Symbol.asyncIterator]() {
      return {
        next() {
          waiting = true;
          return new Promise<IteratorResult<string>>(() => {});
        },
      };
    },
  };
  const shutdown = new AbortController();
  const running = runBatch(session(), { lines, signal: shutdown.signal, write: () => {} });

  await waitFor(() => waiting, "batch input wait");
  const reason = new Error("received SIGTERM");
  shutdown.abort(reason);

  await assert.rejects(running, (error) => error === reason);
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
  assert.equal(opened?.config.maxModelRequests, 3);
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

test("process shutdown cancels the initial resume picker before leaving", async () => {
  const current = session();
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
      assert.fail("shutdown must not open a session");
    },
  };
  const shutdown = new AbortController();
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), {
    ...harness.environment,
    shutdownSignal: shutdown.signal,
  });

  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("saved question"),
    "resume picker",
  );
  shutdown.abort(new Error("received SIGTERM"));
  await running;

  assert.equal(harness.left(), true);
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

test("the TUI completes a large transcript reflow across scheduled frames", async () => {
  const current = session();
  const blocks = Array.from(
    { length: 300 },
    (_, index) => ({ kind: "answer" as const, text: `answer ${index}` }),
  );
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "large transcript" }] },
      { role: "assistant", content: [{ kind: "text", text: "complete" }] },
    ],
    blocks,
  }, "completed");
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  await waitFor(() => harness.frames.length >= 3, "incremental transcript reflow");
  assert.match((harness.frames[0] ?? []).join("\n"), /answer 299/);
  assert.ok(harness.frames.every((frame) => frame.length === 18));
  const settledFrames = harness.frames.length;
  await delay(40);
  assert.equal(harness.frames.length, settledFrames);

  feed("/exit\r");
  await running;
});

test("initial and scheduled paint failures restore every TUI owner", async () => {
  for (const failureAt of [1, 2]) {
    let feed: ((chunk: string) => void) | undefined;
    let paints = 0;
    let left = 0;
    let inputStopped = 0;
    let resizeStopped = 0;
    let persistenceClosed = 0;
    const current = session();
    current.persistence = {
      close: async () => {
        persistenceClosed++;
      },
    } as SessionPersistence;
    const screen: AppScreen = {
      size: () => ({ rows: 18, cols: 70 }),
      enter: () => {},
      leave: () => {
        left++;
      },
      setReducedMotion: () => {},
      onResize: () => () => {
        resizeStopped++;
      },
      onInput: (handler) => {
        feed = handler;
        return () => {
          inputStopped++;
        };
      },
    };
    const paint: Painter = {
      paint: () => {
        paints++;
        if (paints === failureAt) throw new Error("fixture paint failed");
      },
      invalidate: () => {},
    };

    const running = runApp(current, process.cwd(), { screen, paint });
    const rejected = assert.rejects(running, /fixture paint failed/);
    while (feed === undefined) await new Promise<void>((resolve) => setImmediate(resolve));
    if (failureAt === 2) feed("x");
    await rejected;

    assert.equal(left, 1);
    assert.equal(inputStopped, 1);
    assert.equal(resizeStopped, 1);
    assert.equal(persistenceClosed, 1);
  }
});

test("a fatal paint failure aborts active work before persistence closes", async () => {
  let started = (): void => {};
  const providerStarted = new Promise<void>((resolve) => {
    started = resolve;
  });
  let aborted = false;
  let providerSettled = false;
  let persistenceClosedAfterSettlement = false;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      started();
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => {
          aborted = true;
          setTimeout(() => {
            providerSettled = true;
            reject(request.signal?.reason);
          }, 20);
        }, { once: true });
      });
    },
  };
  const current = session(waiting);
  current.persistence = {
    close: async () => {
      persistenceClosedAfterSettlement = providerSettled;
    },
  } as SessionPersistence;
  let paints = 0;
  const harness = virtualScreen();
  harness.environment.paint = {
    paint: (rows) => {
      paints++;
      if (paints === 2) throw new Error("fatal fixture paint failure");
      harness.frames.push([...rows]);
    },
    invalidate: () => {},
  };

  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();
  feed("wait\r");
  await providerStarted;
  await assert.rejects(running, /fatal fixture paint failure/);

  assert.equal(aborted, true);
  assert.equal(providerSettled, true);
  assert.equal(persistenceClosedAfterSettlement, true);
  assert.equal(harness.left(), true);
});

test("teardown releases every owner even when one cleanup callback throws", async () => {
  let feed: ((chunk: string) => void) | undefined;
  let resizeStopped = false;
  let left = false;
  let persistenceClosed = false;
  const current = session();
  current.persistence = {
    close: async () => {
      persistenceClosed = true;
    },
  } as SessionPersistence;
  const screen: AppScreen = {
    size: () => ({ rows: 18, cols: 70 }),
    enter: () => {},
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
        throw new Error("fixture input cleanup failed");
      };
    },
  };
  const paint: Painter = { paint: () => {}, invalidate: () => {} };

  const running = runApp(current, process.cwd(), { screen, paint });
  await waitFor(() => feed !== undefined, "TUI input handler");
  feed?.("/exit\r");
  await assert.rejects(running, /fixture input cleanup failed/);

  assert.equal(resizeStopped, true);
  assert.equal(left, true);
  assert.equal(persistenceClosed, true);
});

test("a streamed failure survives export, resume, and the next provider request", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-failed-turn-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  const requests: Message[][] = [];
  let sends = 0;
  const flaky: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      sends++;
      requests.push(structuredClone(request.messages));
      if (sends === 1) {
        request.onStream?.({ kind: "text", text: "partial answer" });
        throw new Error("fixture stream failed");
      }
      request.onStream?.({ kind: "text", text: "recovered answer" });
      return { role: "assistant", content: [{ kind: "text", text: "recovered answer" }] };
    },
  };

  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const current = session(flaky);
    current.config.root = workspace;
    current.persistence = SessionPersistence.fresh(store);
    const harness = virtualScreen();
    const running = runApp(current, workspace, harness.environment);
    const feed = await harness.input();

    feed("first request\r");
    await waitFor(
      () => current.conversation.activeNode?.settlement === "failed",
      "durable failed turn",
    );
    const sessionId = current.persistence.sessionId;
    assert.ok(sessionId !== null);
    assert.match(JSON.stringify(current.conversation.transcript), /partial answer/);
    assert.match(JSON.stringify(current.conversation.transcript), /fixture stream failed/);

    feed("/export\r");
    await waitFor(async () => (await readdir(workspace)).some((name) => name.startsWith("jecode-transcript-")), "failed turn export");
    const exportedName = (await readdir(workspace)).find((name) => name.startsWith("jecode-transcript-"));
    assert.ok(exportedName !== undefined);
    const exported = await readFile(path.join(workspace, exportedName), "utf8");
    assert.match(exported, /partial answer/);
    assert.match(exported, /fixture stream failed/);

    feed("retry\r");
    await waitFor(
      () => current.conversation.activeNode?.settlement === "completed",
      "completed retry",
    );
    assert.match(JSON.stringify(requests[1]), /first request/);
    assert.match(JSON.stringify(requests[1]), /failed before completion/);
    assert.match(JSON.stringify(requests[1]), /retry/);

    feed("/exit\r");
    await running;
    const resumed = await SessionPersistence.resume(store, sessionId);
    assert.equal(resumed.conversation.nodes.length, 2);
    assert.equal(resumed.conversation.node(1)?.settlement, "failed");
    assert.match(JSON.stringify(resumed.conversation.transcript), /fixture stream failed/);
    await resumed.persistence.close();
  } finally {
    await rm(root, { recursive: true, force: true });
  }
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

test("the footer follows model, tool preparation, execution, and response phases", async () => {
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

test("/compact revises context without adding its summary to the transcript", async () => {
  const requests: SendRequest[] = [];
  const compacting: Provider = {
    ...provider(),
    contextWindow: () => Promise.resolve({ tokens: 64_000 }),
    async send(request): Promise<Message> {
      requests.push(request);
      return {
        role: "assistant",
        content: [{ kind: "text", text: "Manual durable summary." }],
        usage: {
          inputTokens: 12_000,
          outputTokens: 12,
          cachedInputTokens: 0,
          cacheWriteInputTokens: 0,
          reasoningTokens: 0,
        },
      };
    },
  };
  const current = session(compacting);
  const source = "large canonical context ".repeat(2_000);
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: source }] },
      { role: "assistant", content: [{ kind: "text", text: "Canonical answer." }] },
    ],
    blocks: [
      { kind: "user", text: "large canonical context" },
      { kind: "answer", text: "Canonical answer." },
    ],
  }, "completed");
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("/compact\r");
  await waitFor(() => current.conversation.activeNode?.revision === 2, "manual compaction");
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("context compacted"),
    "manual compaction feedback",
  );
  const footer = plainRow(harness.frames.at(-1)?.at(-1) ?? "");
  assert.match(footer, /context compacted$/);
  assert.doesNotMatch(footer, /[.·]\s+context compacted/);
  assert.equal(requests.length, 1);
  assert.match(JSON.stringify(current.conversation.history), /large canonical context/);
  assert.doesNotMatch(JSON.stringify(current.conversation.transcript), /Manual durable summary/);
  assert.match(JSON.stringify(current.conversation.contextHistory), /Manual durable summary/);

  feed("/exit\r");
  await running;
});

test("/timeline creates no branch until the next user turn", async () => {
  const current = session(provider("Alternate answer."));
  const first = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "first request" }] },
      { role: "assistant", content: [{ kind: "text", text: "First answer." }] },
    ],
    blocks: [
      { kind: "user", text: "first request" },
      { kind: "answer", text: "First answer." },
    ],
  }, "completed");
  current.conversation = first.commit({
    parentId: 1,
    createdAt: "2026-09-02T10:01:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "second request" }] },
      { role: "assistant", content: [{ kind: "text", text: "Second answer." }] },
    ],
    blocks: [
      { kind: "user", text: "second request" },
      { kind: "answer", text: "Second answer." },
    ],
  }, "completed");
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("/timeline\r");
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("conversation tree"),
    "timeline picker",
  );
  feed(`${String.fromCharCode(27)}[A\r`);
  await waitFor(() => current.conversation.activeNodeId === 1, "timeline selection");
  await waitFor(
    () => !(harness.frames.at(-1) ?? []).join("\n").includes("Second answer."),
    "selected branch transcript",
  );
  assert.equal(current.conversation.nodes.length, 2);
  assert.doesNotMatch((harness.frames.at(-1) ?? []).join("\n"), /Second answer\./);

  feed("/compact\r");
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("before compacting"),
    "pending branch guard",
  );
  assert.equal(current.conversation.nodes.length, 2);

  feed("alternate request\r");
  await waitFor(() => current.conversation.activeNodeId === 3, "persisted alternate branch");
  assert.equal(current.conversation.node(3)?.parentId, 1);
  assert.deepEqual(
    current.conversation.history.flatMap((message) => message.content)
      .filter((block) => block.kind === "text")
      .map((block) => block.text),
    ["first request", "First answer.", "alternate request", "Alternate answer."],
  );

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

test("process shutdown aborts one active TUI turn and waits for its settlement", async () => {
  let signal: AbortSignal | undefined;
  let checkpoints = 0;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      signal = request.signal;
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const current = session(waiting);
  current.persistence = {
    checkpoint: async () => {
      checkpoints++;
    },
    close: async () => {},
  } as unknown as SessionPersistence;
  const shutdown = new AbortController();
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), {
    ...harness.environment,
    shutdownSignal: shutdown.signal,
  });
  const feed = await harness.input();

  feed("wait\r");
  await waitFor(() => signal !== undefined, "provider request");
  shutdown.abort(new Error("received SIGTERM"));
  await running;

  assert.equal(signal?.aborted, true);
  assert.equal(current.conversation.activeNode?.settlement, "interrupted");
  assert.equal(checkpoints, 1);
  assert.equal(harness.left(), true);
});

test("the TUI steers an active response inside the same durable turn", async () => {
  const first = deferred();
  const requests: Message[][] = [];
  const steeringProvider: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      requests.push(structuredClone(request.messages));
      if (requests.length === 1) {
        await first.wait;
        request.onStream?.({ kind: "text", text: "First answer." });
        return { role: "assistant", content: [{ kind: "text", text: "First answer." }] };
      }
      request.onStream?.({ kind: "text", text: "Revised answer." });
      return { role: "assistant", content: [{ kind: "text", text: "Revised answer." }] };
    },
  };
  const current = session(steeringProvider);
  const harness = virtualScreen(100);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("initial request\r");
  await waitFor(() => requests.length === 1, "first provider request");
  await waitFor(() => lastFooter(harness).includes("enter to steer"), "steering hint");
  feed("change direction\r");
  await waitFor(() => lastFooter(harness).includes("1 queued"), "queued steering footer");
  first.release();

  await waitFor(() => requests.length === 2, "steered provider request");
  await waitFor(
    () => current.conversation.activeNode?.settlement === "completed",
    "completed steered turn",
  );
  assert.equal(current.conversation.nodes.length, 1);
  assert.deepEqual(
    current.conversation.history.flatMap((message) => message.content)
      .filter((block) => block.kind === "text")
      .map((block) => block.text),
    ["initial request", "First answer.", "change direction", "Revised answer."],
  );
  assert.match(JSON.stringify(current.conversation.transcript), /change direction/);

  feed("/exit\r");
  await running;
});

test("interruption restores steering that the model did not receive", async () => {
  const requests: Message[][] = [];
  const waiting: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      requests.push(structuredClone(request.messages));
      if (requests.length > 1) {
        return { role: "assistant", content: [{ kind: "text", text: "Recovered." }] };
      }
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const current = session(waiting);
  const harness = virtualScreen(100);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("wait\r");
  await waitFor(() => requests.length === 1, "waiting provider request");
  feed("recover this guidance\r");
  await waitFor(() => lastFooter(harness).includes("1 queued"), "queued guidance");
  feed(String.fromCharCode(27));
  await waitFor(
    () => current.conversation.activeNode?.settlement === "interrupted" &&
      !lastFooter(harness).includes("esc to interrupt"),
    "settled interruption",
  );
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("recover this guidance"),
    "restored composer guidance",
  );

  feed("\r");
  await waitFor(() => requests.length === 2, "retried guidance request");
  await waitFor(
    () => current.conversation.activeNode?.settlement === "completed",
    "completed recovered guidance",
  );
  assert.equal(messageText(requests[1]?.at(-1)), "recover this guidance");

  feed("/exit\r");
  await running;
});

test("a failed checkpoint returns the original prompt and pending steering", async () => {
  let requested = false;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      requested = true;
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const current = session(waiting);
  current.persistence = {
    checkpoint: async () => {
      throw new Error("fixture checkpoint failed");
    },
    close: async () => {},
  } as unknown as SessionPersistence;
  const harness = virtualScreen(100);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("original prompt\r");
  await waitFor(() => requested, "provider request");
  feed("pending guidance\r");
  await waitFor(() => lastFooter(harness).includes("1 queued"), "queued guidance");
  feed(String.fromCharCode(27));
  await waitFor(
    () => lastFooter(harness).includes("fixture checkpoint failed"),
    "checkpoint failure",
  );

  const frame = (harness.frames.at(-1) ?? []).join("\n");
  assert.match(frame, /original prompt/);
  assert.match(frame, /pending guidance/);

  feed(String.fromCharCode(3));
  await running;
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
  assert.match(result.stdout, /optional per-turn model-request budget/);
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

test("the packaged batch executable rejects an oversized final line on stderr", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-batch-limit-home-"));
  const environment: NodeJS.ProcessEnv = {
    ...process.env,
    HOME: directory,
    USERPROFILE: directory,
    APPDATA: directory,
    LOCALAPPDATA: directory,
    JECODE_HOME: directory,
    XDG_CONFIG_HOME: directory,
  };

  try {
    const result = await runNode(
      path.resolve("bin/jecode.js"),
      ["--provider", "anthropic", "--model", "fixture-model", "--ephemeral"],
      { input: "x".repeat(MAX_PROMPT_CODE_UNITS + 1), environment },
    );

    assert.equal(result.code, 1);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /^jecode: Prompt cannot exceed 1,048,576 UTF-16 code units/m);
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

function virtualScreen(cols = 70): {
  environment: { screen: AppScreen; paint: Painter };
  frames: string[][];
  input(): Promise<(chunk: string) => void>;
  left(): boolean;
} {
  let feed: ((chunk: string) => void) | undefined;
  let left = false;
  const frames: string[][] = [];
  const screen: AppScreen = {
    size: () => ({ rows: 18, cols }),
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

function plainRow(value: string): string {
  return value.replace(/\u001b\[[0-9;]*m/gu, "");
}

function lastFooter(harness: { frames: string[][] }): string {
  return plainRow(harness.frames.at(-1)?.at(-1) ?? "");
}

function deferred(): { wait: Promise<void>; release(): void } {
  let release = (): void => {};
  const wait = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { wait, release };
}
