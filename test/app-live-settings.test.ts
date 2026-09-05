import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { PROVIDERS } from "../src/providers/index.ts";
import { readSettings, reloadSettings } from "../src/settings.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { aborted, assistantText, echo } from "../dev/test-support/controller.ts";
import { deferred, lastFooter, plainRow, virtualScreen, waitFor } from "../dev/test-support/app-harness.ts";

const ESC = "\u001b";
const frame = (screen: { frames: string[][] }) => plainRow(screen.frames.at(-1)?.join("\n") ?? "");

test("model and settings changes apply next turn; active requests and checkpoints stay pinned", async (t) => {
  const home = await mkdtemp(path.join(tmpdir(), "jecode-live-settings-"));
  const previousHome = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = home;
  reloadSettings();
  const first = deferred();
  const second = deferred();
  const originalRequests: SendRequest[] = [];
  const nextRequests: SendRequest[] = [];
  const contextModels: string[] = [];
  const original = { ...provider(),
    async contextWindow(model: string) { contextModels.push(model); return undefined; },
    async send(request: SendRequest): Promise<Message> {
      originalRequests.push(request);
      if (originalRequests.length === 1) {
        await Promise.race([first.wait, aborted(request.signal)]);
        return { role: "assistant", content: [{ kind: "tool_call", id: "read", name: "echo", input: { text: "ok" } }] };
      }
      request.onStream?.({ kind: "text", text: "Original model finished." });
      await Promise.race([second.wait, aborted(request.signal)]);
      return assistantText("Original model finished.");
    },
  };
  const replacement = PROVIDERS[0]!;
  for (const candidate of PROVIDERS) {
    t.mock.method(candidate, "blocked", () => candidate === replacement ? undefined : "fixture disconnected");
  }
  t.mock.method(replacement, "models", async () => ["next-model"]);
  t.mock.method(replacement as { contextWindow: NonNullable<Provider["contextWindow"]> }, "contextWindow", async () => undefined);
  t.mock.method(replacement, "send", async (request: SendRequest) => {
    nextRequests.push(request);
    return assistantText("New model answered.");
  });
  const current = session(original);
  current.tools = [echo];
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  try {
    const feed = await screen.input();
    feed("work\r");
    await waitFor(() => originalRequests.length === 1, "original request");
    feed("/settings\r");
    await waitFor(() => frame(screen).includes("reduced motion"), "settings");
    feed(`${ESC}[B\r`);
    await waitFor(() => frame(screen).includes("● high"), "effort picker");
    feed(`${ESC}[H\r`);
    await waitFor(() => current.config.effort === "low" && frame(screen).includes("reduced motion"), "saved effort");
    feed(`${ESC}[B${ESC}[B\r`);
    await waitFor(() => frame(screen).includes("enter save"), "compaction field");
    feed("\u001590\r");
    await waitFor(() => current.config.compactionPercent === 90 && frame(screen).includes("reduced motion"), "saved threshold");
    feed(`${ESC}[H\r`);
    await waitFor(() => frame(screen).includes("● next-model"), "model picker");
    feed("\r");
    await waitFor(() => current.provider === replacement && frame(screen).includes("reduced motion"), "saved model");
    assert.equal(current.model, "next-model");
    assert.equal(readSettings().models?.[replacement.id], "next-model");
    assert.match(lastFooter(screen), /Fake.*fake-1.*high/);
    feed(ESC);
    await waitFor(() => lastFooter(screen).includes("enter to steer"), "composer");
    feed("continue carefully\r");
    first.release();
    await waitFor(() => originalRequests.length === 2, "same controller continuation");
    assert.equal(nextRequests.length, 0);
    assert.ok(originalRequests.every((request) => request.model === "fake-1" && request.effort === "high"));
    assert.deepEqual(originalRequests[1]?.identity, originalRequests[0]?.identity);
    assert.ok(contextModels.length > 1 && contextModels.every((model) => model === "fake-1"));
    second.release();
    await waitFor(() => current.conversation.activeNode?.settlement === "completed", "settlement");
    assert.deepEqual(current.conversation.activeNode?.identity, { providerId: "fake", model: "fake-1", effort: "high" });
    await waitFor(() => lastFooter(screen).includes("next-model"), "next selection in footer");
    feed("next turn\r");
    await waitFor(() => current.conversation.nodes.length === 2, "next completed turn");
    assert.equal(nextRequests.length, 1);
    assert.equal(nextRequests[0]?.model, "next-model");
    assert.equal(nextRequests[0]?.effort, "low");
    assert.doesNotMatch(JSON.stringify(current.conversation.history), /\/settings/);
  } finally {
    shutdown.abort();
    await running;
    if (previousHome === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = previousHome;
    reloadSettings();
    await rm(home, { recursive: true, force: true });
  }
});

test("cancelling asynchronous menu discovery leaves the model request alive", async (t) => {
  let modelSignal: AbortSignal | undefined;
  let menuSignal: AbortSignal | undefined;
  const current = session({ ...provider(),
    efforts: (_model, signal) => { menuSignal = signal; return aborted(signal); },
    send: (request) => { modelSignal = request.signal; return aborted(request.signal); },
  });
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  t.after(async () => { shutdown.abort(); await running; });
  const feed = await screen.input();
  feed("work\r");
  await waitFor(() => modelSignal !== undefined, "model request");
  feed("/effort\r");
  await waitFor(() => menuSignal !== undefined, "effort discovery");
  feed("\u0003");
  await waitFor(() => menuSignal?.aborted === true, "discovery cancellation");
  await waitFor(() => lastFooter(screen).includes("enter to steer"), "ready composer");
  assert.equal(modelSignal?.aborted, false);
  feed("/help\r");
  await waitFor(() => frame(screen).includes("keyboard controls"), "next command");
});
