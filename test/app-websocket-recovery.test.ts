import { test } from "node:test";
import assert from "node:assert/strict";
import { appendFile, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import type { Provider, SendRequest } from "../src/types.ts";
import { ResponsesSession } from "../src/providers/responses-session.ts";
import { requestResponses } from "../src/providers/responses-request.ts";
import { fromWireResponse, toWireItems, toWireTool } from "../src/providers/openai-wire.ts";
import { normalizeProviderError } from "../src/providers/failure.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import { runApp } from "../src/tui/app.ts";
import { ConversationTree } from "../src/conversation.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor, waitForIdle } from "../dev/test-support/app-harness.ts";
import { completed, responsesServer, socketJson } from "../dev/test-support/responses-server.ts";

for (const mode of ["socket close", "failed terminal after compaction"] as const) {
test(`${mode} preserves completed effects across TUI exit and resume without replay`, { timeout: 15_000 }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-socket-recovery-"));
  const workspace = path.join(root, "workspace");
  await mkdir(workspace);
  const sent: Record<string, unknown>[] = [];
  let summaries = 0;
  let releaseSummary: (() => void) | undefined;
  const call = (id: string) => ({ type: "function_call", id: `fc_${id}`, status: "completed", call_id: id,
    name: "record_effect", arguments: "{}" });
  const server = await responsesServer({ message(body, socket) {
    if (String(body["instructions"]).includes("durable working memory")) {
      summaries++;
      releaseSummary = () => socketJson(socket, completed("memory", "Preserve earlier decisions."));
      return;
    }
    sent.push(body);
    if (sent.length === 1) {
      const event = completed("first");
      socketJson(socket, { ...event, response: { ...event.response, output: [call("committed")] } });
    } else if (sent.length === 2) {
      socketJson(socket, { type: "response.created" });
      // A completed item without a terminal response cannot authorize a tool.
      socketJson(socket, { type: "response.output_item.done", item: call("uncommitted") });
      socketJson(socket, { type: "response.output_text.delta", delta: "Unfinished output." });
      if (mode === "socket close") socket.end();
      else socketJson(socket, { type: "response.completed", response: { status: "failed", output: [],
        error: { code: "server_error", message: "Generation could not complete." } } });
    } else {
      socketJson(socket, completed("recovered", "Completed from saved state."));
    }
  } });
  const send = async (request: SendRequest, transport: ResponsesSession) => {
    try {
      const result = await requestResponses("openai", server.url, {}, {
        input: request.messages.flatMap(message => toWireItems(message)),
        model: request.model, instructions: request.system, tools: request.tools.map(toWireTool), stream: true,
      }, request, transport);
      return fromWireResponse(result);
    } catch (error) { throw normalizeProviderError("openai", error); }
  };
  const wire: Provider = { ...provider(), id: "openai",
    contextWindow: async () => ({ tokens: 64_000 }),
    async send(request) {
      assert.match(request.system, /durable working memory/, "normal generation uses the controller scope");
      const transport = new ResponsesSession();
      try { return await send(request, transport); } finally { transport.close(); }
    },
    openTurn() {
      const transport = new ResponsesSession();
      return { send: request => send(request, transport), close: () => transport.close() };
    },
  };
  const store = await DurableSessionStore.open(workspace, path.join(root, "sessions"));
  const makeSession = () => {
    const current = session(wire);
    current.config.root = workspace;
    current.tools = [{ name: "record_effect", description: "Record one fixture effect.",
      input: { type: "object", properties: {} }, dangerous: false, concurrency: "exclusive",
      async run() { await appendFile(path.join(workspace, "effects.txt"), "effect\n"); return { output: "recorded" }; },
    }];
    return current;
  };
  const shutdown = new AbortController();
  let running: Promise<void> | undefined;
  let resumed: Promise<void> | undefined;
  try {
    const first = makeSession();
    if (mode === "failed terminal after compaction") {
      first.conversation = ConversationTree.empty().commit({ parentId: 0,
        createdAt: "2026-09-08T10:00:00.000Z",
        identity: { providerId: "openai", model: "fake-1", effort: "high" },
        messages: [{ role: "user", content: [{ kind: "text", text: "old context ".repeat(11_000) }] },
          { role: "assistant", content: [{ kind: "text", text: "Earlier answer." }] }], blocks: [],
      }, "completed");
    }
    first.persistence = SessionPersistence.fresh(store);
    const screen = virtualScreen(40);
    running = runApp(first, workspace, { ...screen.environment, shutdownSignal: shutdown.signal });
    const feed = await screen.input();
    feed("record an effect and finish\r");
    if (mode === "failed terminal after compaction") {
      await waitFor(() => releaseSummary !== undefined, "pending compaction", 10_000);
      feed("Preserve the public API.\r");
      releaseSummary!();
    }
    // Real loopback traffic and durable checkpoints share a loaded CI runner.
    // This checks recovery correctness, not a two-second performance contract.
    await waitFor(() => first.conversation.activeNode?.settlement === "failed", "failed socket turn", 10_000);
    await waitForIdle(screen, "failed socket turn idle");
    assert.equal(sent.length, 2);
    assert.equal(await readFile(path.join(workspace, "effects.txt"), "utf8"), "effect\n");
    assert.match(first.conversation.activeNode!.failure!.text,
      mode === "socket close" ? /stream disconnected.*send a message to continue/ : /Generation could not complete/);
    if (mode === "failed terminal after compaction") {
      assert.equal(summaries, 1);
      assert.match(JSON.stringify(sent[0]!["input"]), /Preserve earlier decisions.*Preserve the public API/);
      assert.doesNotMatch(JSON.stringify(sent[0]!["input"]), /old context/);
      assert.equal(first.conversation.history.filter(message =>
        message.content.some(block => block.kind === "text" && block.text === "Preserve the public API.")).length, 1);
      assert.match(JSON.stringify(first.conversation.history), /old context/);
      assert.ok(first.conversation.activeNode?.context);
    }
    assert.match(JSON.stringify(first.conversation.transcript), /Unfinished output/);
    const id = first.persistence.sessionId!;
    feed("/exit\r");
    await running;

    const saved = await SessionPersistence.resume(store, id);
    const originals = structuredClone(saved.conversation.nodes);
    const next = makeSession();
    next.persistence = saved.persistence;
    next.conversation = saved.conversation;
    const resumedScreen = virtualScreen(40);
    resumed = runApp(next, workspace, { ...resumedScreen.environment, shutdownSignal: shutdown.signal });
    const resumeFeed = await resumedScreen.input();
    await waitForIdle(resumedScreen, "resumed idle before new input");
    assert.equal(sent.length, 2, "resume alone does not generate or replay tools");
    resumeFeed("continue from the saved result\r");
    await waitFor(() => next.conversation.activeNode?.settlement === "completed", "recovered socket turn", 10_000);
    await waitForIdle(resumedScreen, "recovered idle");
    assert.equal(sent.length, 3);
    assert.equal(sent[1]!["previous_response_id"], "first");
    assert.equal(sent[2]!["previous_response_id"], undefined);
    const input = JSON.stringify(sent[2]!["input"]);
    assert.match(input, /recorded/);
    assert.match(input, /failed before completion/);
    assert.doesNotMatch(input, /uncommitted|Unfinished output/);
    if (mode === "failed terminal after compaction") {
      assert.match(input, /Preserve earlier decisions.*Preserve the public API/);
      assert.doesNotMatch(input, /old context/);
      assert.equal(summaries, 1, "resume reuses the durable anchor");
    }
    assert.equal(await readFile(path.join(workspace, "effects.txt"), "utf8"), "effect\n");
    assert.deepEqual(next.conversation.nodes.slice(0, originals.length), originals);
    assert.deepEqual(server.counts(), { upgrades: 2 + summaries, posts: 0 });
    resumeFeed("/exit\r");
    await resumed;
  } finally {
    // Failed assertions must stop in-flight work before its files are removed.
    shutdown.abort();
    await Promise.allSettled([running, resumed]);
    await server.close();
    await rm(root, { recursive: true, force: true });
  }
});
}
