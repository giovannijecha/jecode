import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, SendRequest } from "../src/types.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { aborted, assistantText, destroy, echo } from "../dev/test-support/controller.ts";
import { deferred, lastFooter, plainRow, virtualScreen, waitFor } from "../dev/test-support/app-harness.ts";

const ESC = "\u001b";
const frame = (screen: { frames: string[][] }) => plainRow(screen.frames.at(-1)?.join("\n") ?? "");

test("a menu survives streamed output and turn settlement without losing selection", async (t) => {
  const response = deferred();
  let request: SendRequest | undefined;
  const current = session({ ...provider(), async send(next) {
    request = next;
    await Promise.race([response.wait, aborted(next.signal)]);
    next.onStream?.({ kind: "text", text: "Finished while browsing." });
    return assistantText("Finished while browsing.");
  } });
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  t.after(async () => { shutdown.abort(); await running; });
  const feed = await screen.input();
  feed("work\r");
  await waitFor(() => request !== undefined, "request");
  feed("/settings\r");
  await waitFor(() => frame(screen).includes("reduced motion"), "settings menu");
  feed(`${ESC}[B`);
  await waitFor(() => frame(screen).includes("● effort"), "effort selection");
  request?.onStream?.({ kind: "text", text: "Still working." });
  await waitFor(() => frame(screen).includes("Still working."), "stream beneath menu");
  assert.match(frame(screen), /● effort/);
  assert.equal(request?.signal?.aborted, false);
  response.release();
  await waitFor(() => current.conversation.activeNode?.settlement === "completed", "settled turn");
  await waitFor(() => frame(screen).includes("Finished while browsing."), "settled frame");
  assert.match(frame(screen), /● effort/);
  feed(ESC);
  await waitFor(() => !frame(screen).includes("reduced motion"), "closed settings");
  feed("/exit\r");
  await running;
});

test("approvals take focus and restore the menu after an explicit answer", async (t) => {
  const first = deferred();
  const last = deferred();
  let requests = 0;
  let executions = 0;
  const current = session({ ...provider(), async send(request): Promise<Message> {
    if (++requests === 1) {
      await Promise.race([first.wait, aborted(request.signal)]);
      return { role: "assistant", content: [{ kind: "tool_call", id: "write", name: "destroy", input: {} }] };
    }
    await Promise.race([last.wait, aborted(request.signal)]);
    return assistantText("Done.");
  } });
  current.tools = [{ ...destroy, async run() { executions++; return { output: "done" }; } }];
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  t.after(async () => { shutdown.abort(); await running; });
  const feed = await screen.input();
  feed("work\r");
  await waitFor(() => requests === 1, "first request");
  feed("/settings\r");
  await waitFor(() => frame(screen).includes("reduced motion"), "settings");
  feed(`${ESC}[B`);
  first.release();
  await waitFor(() => frame(screen).includes("Yes, once"), "approval");
  assert.equal(executions, 0);
  assert.doesNotMatch(frame(screen), /reduced motion/);
  feed("y");
  await waitFor(() => requests === 2 && frame(screen).includes("● effort"), "restored settings");
  assert.equal(executions, 1);
  last.release();
  await waitFor(() => current.conversation.activeNode?.settlement === "completed", "completed");
  assert.match(frame(screen), /● effort/);
});

test("Ctrl+C cancels a menu without interrupting the background turn", async (t) => {
  let request: SendRequest | undefined;
  const current = session({ ...provider(), send(next) { request = next; return aborted(next.signal); } });
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  t.after(async () => { shutdown.abort(); await running; });
  const feed = await screen.input();
  feed("work\r");
  await waitFor(() => request !== undefined, "request");
  feed("/help\r");
  await waitFor(() => frame(screen).includes("keyboard controls"), "help");
  feed("\u0003");
  await waitFor(() => !frame(screen).includes("keyboard controls"), "closed help");
  assert.equal(request?.signal?.aborted, false);
  feed("guidance\r");
  await waitFor(() => lastFooter(screen).includes("1 queued"), "steering after closing help");
  feed("/exit\r");
  await running;
  assert.equal(request?.signal?.aborted, true);
  assert.equal(current.conversation.activeNode?.settlement, "interrupted");
});

test("process shutdown cancels both an approval and its suspended command", async () => {
  const ready = deferred();
  let requests = 0;
  let executions = 0;
  let closedAfterSettlement = false;
  const current = session({ ...provider(), async send(request): Promise<Message> {
    requests++;
    await Promise.race([ready.wait, aborted(request.signal)]);
    return { role: "assistant", content: [{ kind: "tool_call", id: "write", name: "destroy", input: {} }] };
  } });
  current.tools = [{ ...destroy, async run() { executions++; return { output: "done" }; } }];
  current.persistence = {
    checkpoint: async () => {},
    close: async () => { closedAfterSettlement = current.conversation.activeNode?.settlement === "interrupted"; },
  } as unknown as NonNullable<typeof current.persistence>;
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  try {
    const feed = await screen.input();
    feed("work\r");
    await waitFor(() => requests === 1, "request");
    feed("/settings\r");
    await waitFor(() => frame(screen).includes("reduced motion"), "settings");
    ready.release();
    await waitFor(() => frame(screen).includes("Yes, once"), "approval");
  } finally {
    shutdown.abort();
    await running;
  }
  assert.equal(executions, 0);
  assert.equal(closedAfterSettlement, true);
  assert.equal(screen.left(), true);
});

test("permission changes deny advertised reads before execution during a turn", async (t) => {
  const response = deferred();
  let requests = 0;
  let executions = 0;
  const current = session({ ...provider(), async send(request): Promise<Message> {
    if (++requests > 1) return assistantText("Access denied.");
    await Promise.race([response.wait, aborted(request.signal)]);
    return { role: "assistant", content: [{ kind: "tool_call", id: "read", name: "echo", input: { text: "secret" } }] };
  } });
  current.tools = [{ ...echo, async run() { executions++; return { output: "unexpected" }; } }];
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  t.after(async () => { shutdown.abort(); await running; });
  const feed = await screen.input();
  feed("work\r");
  await waitFor(() => requests === 1, "request");
  feed("/permissions\r");
  await waitFor(() => frame(screen).includes("‹ allow ›"), "permissions");
  feed(`${ESC}[C`);
  await waitFor(() => frame(screen).includes("‹ deny ›"), "revoked access");
  response.release();
  await waitFor(() => current.conversation.activeNode?.settlement === "completed", "settled denial");
  assert.equal(executions, 0);
  assert.match(JSON.stringify(current.conversation.history), /denied by the current session permissions/);
  assert.doesNotMatch(frame(screen), /Yes, once/);
});

test("shutdown rejects new menus while the interrupted turn is still settling", async () => {
  const settle = deferred();
  let modelSignal: AbortSignal | undefined;
  const current = session({ ...provider(), async send(request): Promise<Message> {
    modelSignal = request.signal;
    try { return await aborted(request.signal); }
    finally { await settle.wait; }
  } });
  const screen = virtualScreen(100);
  const shutdown = new AbortController();
  const running = runApp(current, process.cwd(), { ...screen.environment, shutdownSignal: shutdown.signal });
  try {
    const feed = await screen.input();
    feed("work\r");
    await waitFor(() => modelSignal !== undefined, "request");
    feed("\u0004/settings\r");
    await waitFor(() => modelSignal?.aborted === true, "shutdown requested");
    settle.release();
    await waitFor(() => screen.left(), "exit without waiting for a newly opened menu");
    assert.ok(screen.frames.every((rows) => !rows.join("\n").includes("reduced motion")));
    assert.equal(current.conversation.activeNode?.settlement, "interrupted");
  } finally {
    settle.release();
    shutdown.abort();
    await running;
  }
});
