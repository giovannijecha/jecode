import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import type { Session } from "../src/session.ts";
import { start } from "../src/start.ts";
import { reloadSettings } from "../src/settings.ts";
import { messageText } from "../dev/test-support/app.ts";

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

test("startup rejects retired endpoints before opening a surface or making a provider request", async () => {
  const home = await mkdtemp(path.join(tmpdir(), "jecode-start-cloud-"));
  const beforeHome = process.env["JECODE_HOME"];
  const beforeHost = process.env["OLLAMA_HOST"];
  const previousFetch = globalThis.fetch;
  process.env["JECODE_HOME"] = home;
  globalThis.fetch = async () => { assert.fail("retired endpoints must not make requests"); };
  const args = ["--provider", "ollama", "--model", "fixture-model", "--ephemeral"];
  const environment = {
    interactive: () => true,
    runInteractive: async () => { assert.fail("retired endpoints must stop before the TUI opens"); },
  };
  try {
    reloadSettings();
    process.env["OLLAMA_HOST"] = "http://127.0.0.1:11434";
    await assert.rejects(start(args, environment), /OLLAMA_HOST.*remove it/);
    delete process.env["OLLAMA_HOST"];
    const legacy = JSON.stringify({ provider: "ollama", ollamaHost: "https://custom.example.test", effort: "medium" });
    const file = path.join(home, "settings.json");
    await writeFile(file, legacy, "utf8");
    reloadSettings();
    await assert.rejects(start(args, environment), /retired Ollama endpoint.*remove ollamaHost/);
    assert.equal(await readFile(file, "utf8"), legacy);

    await writeFile(file, JSON.stringify({ ollamaHost: "https://ollama.com" }), "utf8");
    reloadSettings();
    let opened = false;
    await start(args, { interactive: () => true, runInteractive: async (session) => {
      opened = true;
      assert.equal(session.provider.id, "ollama");
      assert.equal("ollamaHost" in session.config, false);
    } });
    assert.equal(opened, true);
  } finally {
    globalThis.fetch = previousFetch;
    if (beforeHome === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = beforeHome;
    if (beforeHost === undefined) delete process.env["OLLAMA_HOST"];
    else process.env["OLLAMA_HOST"] = beforeHost;
    reloadSettings();
    await rm(home, { recursive: true, force: true });
  }
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
