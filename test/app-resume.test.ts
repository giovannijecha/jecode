import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor } from "../dev/test-support/app-harness.ts";

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
