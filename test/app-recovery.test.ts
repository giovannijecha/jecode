import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import type { Message, Provider } from "../src/types.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session, messageText } from "../dev/test-support/app.ts";
import {
  virtualScreen,
  waitFor,
  waitForIdle,
  waitForExportCompletion,
} from "../dev/test-support/app-harness.ts";

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

    await waitForIdle(harness, "settled failed turn before export");
    const exportStartedAt = harness.frames.length;
    feed("/export\r");
    await waitFor(async () => (await readdir(workspace)).some((name) => name.startsWith("jecode-transcript-")), "failed turn export");
    const exportedName = (await readdir(workspace)).find((name) => name.startsWith("jecode-transcript-"));
    assert.ok(exportedName !== undefined);
    const exported = await readFile(path.join(workspace, exportedName), "utf8");
    assert.match(exported, /partial answer/);
    assert.match(exported, /fixture stream failed/);
    await waitForExportCompletion(harness, exportStartedAt, "failed-turn export");

    feed("retry\r");
    await waitFor(
      () => current.conversation.activeNode?.settlement === "completed",
      "completed retry",
    );
    await waitForIdle(harness, "settled retry before exit");
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

test("an interrupted turn agrees across screen, export, resume, and the next request", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-interrupted-turn-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  const requests: Message[][] = [];
  let firstSignal: AbortSignal | undefined;
  const recoverable: Provider = {
    ...provider(),
    send(request): Promise<Message> {
      requests.push(structuredClone(request.messages));
      if (requests.length === 1) {
        firstSignal = request.signal;
        return new Promise<Message>((_resolve, reject) => {
          request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
        });
      }
      request.onStream?.({ kind: "text", text: "Recovered answer." });
      return Promise.resolve({
        role: "assistant",
        content: [{ kind: "text", text: "Recovered answer." }],
      });
    },
  };

  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const current = session(recoverable);
    current.config.root = workspace;
    current.persistence = SessionPersistence.fresh(store);
    const harness = virtualScreen(120);
    const running = runApp(current, workspace, harness.environment);
    const feed = await harness.input();

    feed("first request\r");
    await waitFor(() => firstSignal !== undefined, "first provider request");
    feed(String.fromCharCode(27));
    await waitFor(
      () => current.conversation.activeNode?.settlement === "interrupted",
      "durable interrupted turn",
    );
    const sessionId = current.persistence.sessionId;
    assert.ok(sessionId !== null);
    await waitFor(
      () => harness.frames.flat().join("\n").includes("[interrupted]"),
      "painted interrupted turn",
    );

    await waitForIdle(harness, "settled interrupted turn before export");
    const exportStartedAt = harness.frames.length;
    feed("/export\r");
    await waitFor(
      async () => (await readdir(workspace)).some((name) => name.startsWith("jecode-transcript-")),
      "interrupted turn export",
    );
    const exportedName = (await readdir(workspace)).find((name) =>
      name.startsWith("jecode-transcript-")
    );
    assert.ok(exportedName !== undefined);
    const exported = await readFile(path.join(workspace, exportedName), "utf8");
    assert.match(exported, /first request/);
    assert.match(exported, /\[interrupted\]/);
    await waitForExportCompletion(harness, exportStartedAt, "interrupted-turn export");

    feed("continue\r");
    await waitFor(
      () => current.conversation.activeNode?.settlement === "completed",
      "completed recovery turn",
    );
    await waitForIdle(harness, "settled recovery turn before exit");
    assert.equal(requests.length, 2);
    assert.match(JSON.stringify(requests[1]), /first request/);
    assert.match(JSON.stringify(requests[1]), /interrupted by the user before completion/);
    assert.match(JSON.stringify(requests[1]), /continue/);

    feed("/exit\r");
    await running;
    const resumed = await SessionPersistence.resume(store, sessionId);
    assert.equal(resumed.conversation.nodes.length, 2);
    assert.equal(resumed.conversation.node(1)?.settlement, "interrupted");
    assert.equal(resumed.conversation.node(2)?.settlement, "completed");
    assert.equal(
      resumed.conversation.transcript.filter((block) =>
        block.kind === "notice" && block.text === "[interrupted]"
      ).length,
      1,
    );
    await resumed.persistence.close();
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a max-steps failure stays actionable, durable, and recoverable", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-max-steps-turn-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  const requests: Message[][] = [];
  const looping: Provider = {
    ...provider(),
    send(request): Promise<Message> {
      requests.push(structuredClone(request.messages));
      if (messageText(request.messages.at(-1)) === "continue") {
        request.onStream?.({ kind: "text", text: "Recovered after the budget." });
        return Promise.resolve({
          role: "assistant",
          content: [{ kind: "text", text: "Recovered after the budget." }],
        });
      }
      return Promise.resolve({
        role: "assistant",
        content: [{
          kind: "tool_call",
          id: `missing-${requests.length}`,
          name: "missing",
          input: {},
        }],
      });
    },
  };

  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const current = session(looping);
    current.config.root = workspace;
    current.config.maxModelRequests = 2;
    current.persistence = SessionPersistence.fresh(store);
    const harness = virtualScreen(120);
    const running = runApp(current, workspace, harness.environment);
    const feed = await harness.input();

    feed("loop\r");
    await waitFor(
      () => current.conversation.activeNode?.settlement === "failed",
      "durable max-steps failure",
    );
    await waitFor(
      () => harness.frames.flat().join("\n").includes(
        "stopped after 2 model requests (--max-steps limit reached)",
      ),
      "painted max-steps failure",
    );
    const sessionId = current.persistence.sessionId;
    assert.ok(sessionId !== null);
    assert.equal(requests.length, 2);
    assert.match(
      harness.frames.flat().join("\n"),
      /stopped after 2 model requests \(--max-steps limit reached\)/,
    );
    assert.match(
      current.conversation.activeNode?.failure?.text ?? "",
      /--max-steps limit reached/,
    );

    feed("continue\r");
    await waitFor(
      () => current.conversation.activeNode?.settlement === "completed",
      "turn after max-steps failure",
    );
    assert.equal(requests.length, 3);
    assert.equal(current.conversation.nodes.length, 2);
    assert.equal(current.conversation.node(1)?.settlement, "failed");
    assert.equal(current.conversation.node(2)?.settlement, "completed");
    assert.match(JSON.stringify(requests[2]), /failed before completion/);
    assert.match(JSON.stringify(requests[2]), /continue/);

    feed("/exit\r");
    await running;
    const resumed = await SessionPersistence.resume(store, sessionId);
    assert.equal(resumed.conversation.nodes.length, 2);
    assert.equal(resumed.conversation.node(1)?.settlement, "failed");
    assert.equal(resumed.conversation.node(2)?.settlement, "completed");
    assert.match(JSON.stringify(resumed.conversation.transcript), /--max-steps limit reached/);
    await resumed.persistence.close();
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
