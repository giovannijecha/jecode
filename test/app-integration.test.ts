import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { runBatch } from "../src/batch.ts";
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
    keyVar: "FAKE_API_KEY",
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
      root: process.cwd(),
      autoApprove: false,
    },
    provider: from,
    model: from.defaultModel,
    palette: STEEL,
    tools: [],
    system: "be useful",
    history: [],
    usage: emptyUsage(),
  };
}

async function* input(...lines: string[]): AsyncIterable<string> {
  for (const line of lines) yield line;
}

test("batch mode carries input through the controller, renderer, commands, and exit", async () => {
  const output: string[] = [];
  const current = session();

  await runBatch(current, {
    lines: input("hello", "/usage", "/exit", "ignored"),
    width: 60,
    write: (text) => output.push(text),
  });

  const shown = output.join("");
  assert.match(shown, /> hello/);
  assert.match(shown, /Hello from fake\./);
  assert.match(shown, /requests\s+1/);
  assert.doesNotMatch(shown, /ignored/);
  assert.equal(current.history.length, 2);
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
  await waitFor(() => current.history.length === 2, "completed TUI turn");
  await waitFor(
    () => harness.frames.flat().join("\n").includes("Answer from the TUI."),
    "painted TUI answer",
  );
  assert.equal(current.history[0]?.role, "user");
  assert.equal(current.history[1]?.role, "assistant");
  assert.match(harness.frames.flat().join("\n"), /Answer from the TUI\./);

  feed("/exit\r");
  await running;
  assert.equal(harness.left(), true);
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

test("the packaged jecode executable reaches batch help without a developer script", async () => {
  const result = await runExecutable(["--help"], "");
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /Usage:\s+jecode \[options\]/);
  assert.match(result.stdout, /--provider/);
  assert.match(result.stdout, /type \/ to discover/);
  assert.equal(result.stderr, "");
});

test("the packaged executable reports its manifest version", async () => {
  const result = await runExecutable(["--version"], "");
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout.trim(), "0.1.0");
});

function runExecutable(args: string[], stdin: string): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.resolve("bin/jecode.js"), ...args], {
      cwd: process.cwd(),
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
    child.once("error", reject);
    child.once("close", (code) => resolve({ code, stdout, stderr }));
    child.stdin.end(stdin);
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
