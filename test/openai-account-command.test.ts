import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import {
  openAICodexAccount,
  reloadAccounts,
  updateOpenAICodexAccount,
  type OpenAICodexAccount,
} from "../src/accounts.ts";
import type { Host } from "../src/commands.ts";
import {
  ensureOpenAIAccount,
  openAIAccountCommand,
  type OpenAIAccountCommandDependencies,
} from "../src/openai-account-command.ts";
import type { OpenAILogin } from "../src/openai-oauth.ts";
import type { Session } from "../src/session.ts";
import type { Message, SendRequest } from "../src/types.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

const ACCOUNT: OpenAICodexAccount = {
  accessToken: "access-token",
  refreshToken: "refresh-token",
  expiresAt: 2_000_000_000_000,
  accountId: "account-1",
  email: "person@example.test",
};

test("headless sign-in defaults to device code and dismisses its wait panel on success", async () => {
  await inStore(async () => {
    const pickers: Parameters<NonNullable<Host["choose"]>>[0][] = [];
    const opened: string[] = [];
    let waiting: ((index?: number) => void) | undefined;
    let closed = false;
    const host: Host = {
      emit: () => {},
      choose: (picker) => {
        pickers.push(picker);
        if (pickers.length === 1) return Promise.resolve(picker.index);
        return new Promise((resolve) => { waiting = resolve; });
      },
      dismiss: () => waiting?.(undefined),
    };
    const login: OpenAILogin = {
      url: "https://auth.openai.com/codex/device",
      code: "ABCD-EFGH",
      complete: async () => ACCOUNT,
      close: async () => { closed = true; },
    };
    const dependencies = deps(login, opened, true);

    assert.equal(await ensureOpenAIAccount(session(), host, dependencies), true);
    assert.equal(pickers[0]?.index, 1);
    assert.equal(pickers[0]?.options[0]?.hint, "desktop terminal");
    assert.equal(pickers[0]?.options[1]?.hint, "WSL, SSH, or headless");
    assert.match(pickers[0]?.description ?? "", /password stays on OpenAI's website/);
    assert.match(pickers[1]?.description ?? "", /ABCD-EFGH/);
    assert.doesNotMatch(pickers[1]?.description ?? "", /https?:\/\//);
    assert.equal(pickers[1]?.options[1]?.label, "cancel sign-in");
    assert.deepEqual(opened, [login.url]);
    assert.deepEqual(openAICodexAccount(), ACCOUNT);
    assert.equal(closed, true);
  });
});

test("cancelling the wait panel aborts and leaves the account disconnected", async () => {
  await inStore(async () => {
    let closed = false;
    const host: Host = {
      emit: () => {},
      choose: (() => {
        const answers = [0, 1];
        return () => Promise.resolve(answers.shift());
      })(),
    };
    const login: OpenAILogin = {
      url: "https://auth.openai.com/oauth/authorize",
      complete: (signal) => new Promise((_resolve, reject) => {
        signal?.addEventListener("abort", () => reject(signal.reason), { once: true });
      }),
      close: async () => { closed = true; },
    };

    assert.equal(await ensureOpenAIAccount(session(), host, deps(login, [], false)), false);
    assert.equal(openAICodexAccount(), undefined);
    assert.equal(closed, true);
  });
});

test("sign out revokes remotely and removes the saved ChatGPT account", async (context) => {
  await inStore(async () => {
    await updateOpenAICodexAccount(async () => ACCOUNT);
    const previousFetch = globalThis.fetch;
    let body = "";
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      body = String(init?.body ?? "");
      return new Response("{}", { status: 200 });
    }) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });
    const notices: string[] = [];
    const host: Host = {
      emit: (block) => notices.push(block.text),
      choose: () => Promise.resolve(1),
    };

    assert.equal(await openAIAccountCommand(session(), host), true);
    assert.equal(openAICodexAccount(), undefined);
    assert.match(body, /"token":"refresh-token"/);
    assert.deepEqual(notices, ["ChatGPT disconnected"]);
  });
});

function deps(
  login: OpenAILogin,
  opened: string[],
  headless: boolean,
): OpenAIAccountCommandDependencies {
  return {
    beginBrowser: async () => login,
    beginDevice: async () => login,
    openUrl: async (url) => { opened.push(url); return true; },
    headless: () => headless,
  };
}

function session(): Session {
  return {
    config: {
      root: process.cwd(),
      providerId: "openai-codex",
      model: "",
      maxTokens: 64_000,
      maxSteps: 40,
      compactionPercent: 85,
      effort: "high",
      autoApprove: false,
      ephemeral: false,
      reducedMotion: false,
    },
    provider: {
      id: "openai-codex",
      defaultModel: "",
      auth: { kind: "oauth", account: "openai-codex", label: "ChatGPT" },
      blocked: () => openAICodexAccount() === undefined ? "ChatGPT account is not connected" : undefined,
      models: () => Promise.resolve([]),
      send: (_request: SendRequest): Promise<Message> => Promise.reject(new Error("not called")),
    },
    model: "",
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}

async function inStore(body: () => Promise<void>): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-openai-account-command-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadAccounts();
  try {
    await body();
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadAccounts();
    await rm(directory, { recursive: true, force: true });
  }
}
