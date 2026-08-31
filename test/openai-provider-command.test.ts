import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { reloadAccounts, updateOpenAICodexAccount } from "../src/accounts.ts";
import { handleCommand, type Host } from "../src/commands.ts";
import { reloadSettings, readSettings, updateSettings } from "../src/settings.ts";
import type { Session } from "../src/session.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

test("selecting OpenAI Codex authenticates, picks a live model, and saves both", async (context) => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-openai-provider-command-"));
  const beforeHome = process.env["JECODE_HOME"];
  const previousFetch = globalThis.fetch;
  process.env["JECODE_HOME"] = directory;
  reloadAccounts();
  reloadSettings();
  context.after(async () => {
    globalThis.fetch = previousFetch;
    if (beforeHome === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = beforeHome;
    reloadAccounts();
    reloadSettings();
    await rm(directory, { recursive: true, force: true });
  });
  await updateOpenAICodexAccount(async () => ({
    accessToken: "access-token",
    refreshToken: "refresh-token",
    expiresAt: Date.now() + 3_600_000,
    accountId: "account-1",
  }));
  globalThis.fetch = (async () => new Response(JSON.stringify({
    models: [{ slug: "gpt-codex", visibility: "list", priority: 1 }],
  }), { status: 200 })) as typeof fetch;
  const answers = [2, 0];
  const host: Host = {
    emit: () => {},
    choose: () => Promise.resolve(answers.shift()),
    saveSettings: async (patch) => { await updateSettings(patch); },
  };
  const live = session();

  await handleCommand("/providers", live, host);

  assert.equal(live.provider.id, "openai-codex");
  assert.equal(live.model, "gpt-codex");
  assert.equal(readSettings().provider, "openai-codex");
  assert.equal(readSettings().models?.["openai-codex"], "gpt-codex");
});

function session(): Session {
  const provider: Provider = {
    id: "fake",
    defaultModel: "fake-model",
    auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve(["fake-model"]),
    send: (_request: SendRequest): Promise<Message> => Promise.reject(new Error("not called")),
  };
  return {
    config: {
      root: process.cwd(),
      providerId: provider.id,
      model: provider.defaultModel,
      maxTokens: 64_000,
      maxSteps: 40,
      effort: "high",
      autoApprove: false,
      reducedMotion: false,
    },
    provider,
    model: provider.defaultModel,
    palette: STEEL,
    tools: [],
    system: "",
    history: [],
    usage: emptyUsage(),
  };
}
