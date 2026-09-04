import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  openAICodexAccount,
  reloadAccounts,
  updateOpenAICodexAccount,
} from "../src/accounts.ts";
import type { StreamEvent } from "../src/types.ts";
import { openaiCodex } from "../src/providers/openai-codex.ts";

const CLAIMS = "https://api.openai.com/auth";

test("the ChatGPT catalogue is authenticated, visible-only, ordered, and bounded", async (context) => {
  await inStore(async () => {
    await saveAccount("catalog-access", "catalog-refresh");
    const previousFetch = globalThis.fetch;
    let seenUrl = "";
    let seenHeaders: Headers | undefined;
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      seenUrl = String(input);
      seenHeaders = new Headers(init?.headers);
      return json({ models: [
        {
          slug: "later",
          visibility: "list",
          priority: 20,
          context_window: 200_000,
          supported_reasoning_levels: [{ effort: "low" }, { effort: "high" }],
        },
        {
          slug: "first",
          visibility: "list",
          priority: 10,
          context_window: 1_050_000,
          effective_context_window_percent: 95,
          auto_compact_token_limit: 900_000,
          supported_reasoning_levels: [{ effort: "low" }, { effort: "xhigh" }],
        },
        { slug: "private", visibility: "hidden", priority: 0 },
        { slug: "first", visibility: "list", priority: 1 },
      ] });
    }) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });

    assert.deepEqual(await openaiCodex.models(), ["first", "later"]);
    assert.equal(seenUrl, "https://chatgpt.com/backend-api/codex/models?client_version=99.99.99");
    assert.equal(seenHeaders?.get("authorization"), "Bearer catalog-access");
    assert.equal(seenHeaders?.get("chatgpt-account-id"), "account-1");
    assert.equal(seenHeaders?.get("originator"), "jecode");
    assert.match(seenHeaders?.get("user-agent") ?? "", /^jecode\//);
    assert.deepEqual(await openaiCodex.efforts?.("first"), ["low", "xhigh"]);
    assert.deepEqual(await openaiCodex.contextWindow?.("first"), {
      tokens: 997_500,
      compactAtTokens: 900_000,
    });
  });
});

test("the ChatGPT catalogue retains a minimum 4K context after headroom", async (context) => {
  await inStore(async () => {
    await saveAccount("small-context-access", "small-context-refresh");
    const previousFetch = globalThis.fetch;
    globalThis.fetch = (async () => json({ models: [{
      slug: "small-context-model",
      visibility: "list",
      priority: 1,
      context_window: 4_096,
      effective_context_window_percent: 95,
    }] })) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });

    assert.deepEqual(await openaiCodex.contextWindow?.("small-context-model"), {
      tokens: 3_891,
      compactAtTokens: 3_686,
    });
  });
});

test("the ChatGPT provider rejects an effort omitted by the live model catalogue", async (context) => {
  await inStore(async () => {
    await saveAccount("catalog-access", "catalog-refresh");
    const previousFetch = globalThis.fetch;
    globalThis.fetch = (async (input: string | URL | Request) => {
      if (String(input).includes("/models?")) {
        return json({ models: [{
          slug: "limited-model",
          visibility: "list",
          priority: 1,
          supported_reasoning_levels: [
            { effort: "low" },
            { effort: "medium" },
            { effort: "high" },
            { effort: "xhigh" },
          ],
        }] });
      }
      throw new Error("a rejected effort must not reach the response endpoint");
    }) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });

    assert.deepEqual(await openaiCodex.models(), ["limited-model"]);
    await assert.rejects(
      openaiCodex.send({
        model: "limited-model",
        system: "be useful",
        messages: [],
        tools: [],
        maxTokens: 100,
        effort: "max",
      }),
      /does not support effort "max"/,
    );
  });
});

test("the ChatGPT provider sends a stateless Codex response without API token settings", async (context) => {
  await inStore(async () => {
    await saveAccount("send-access", "send-refresh");
    const previousFetch = globalThis.fetch;
    let body: Record<string, unknown> = {};
    let headers = new Headers();
    const statuses: string[] = [];
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      body = JSON.parse(String(init?.body)) as Record<string, unknown>;
      headers = new Headers(init?.headers);
      return sseEvents([
        { type: "response.created", response: {} },
        {
          type: "response.completed",
          response: {
            status: "completed",
            output: [{ type: "message", content: [{ type: "output_text", text: "done" }] }],
          },
        },
      ]);
    }) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });

    const message = await openaiCodex.send({
      model: "gpt-5.6-luna",
      system: "be useful",
      messages: [{ role: "user", content: [{ kind: "text", text: "hello" }] }],
      tools: [],
      maxTokens: 123,
      effort: "max",
      identity: {
        conversationId: "11111111-1111-4111-8111-111111111111",
        cacheKey: "jecode-stable-cache",
        purpose: "turn",
      },
      onStatus: (status) => statuses.push(status),
    });

    assert.deepEqual(message.content, [{ kind: "text", text: "done" }]);
    assert.equal(message.rawFrom, "openai-codex");
    assert.equal(body["max_output_tokens"], undefined);
    assert.equal(body["store"], false);
    assert.equal(body["stream"], true);
    assert.deepEqual(body["reasoning"], { effort: "max", summary: "auto" });
    assert.deepEqual(body["include"], ["reasoning.encrypted_content"]);
    assert.equal(body["prompt_cache_key"], "jecode-stable-cache");
    assert.equal(headers.get("authorization"), "Bearer send-access");
    assert.equal(headers.get("session-id"), "11111111-1111-4111-8111-111111111111");
    assert.equal(headers.get("openai-beta"), "responses=experimental");
    assert.deepEqual(statuses, ["Connecting", "Waiting for model", "Working"]);
  });
});

test("the ChatGPT provider retains a streamed tool call when completed output is empty", async (context) => {
  await inStore(async () => {
    await saveAccount("send-access", "send-refresh");
    const previousFetch = globalThis.fetch;
    globalThis.fetch = (async () => sseEvents([
      {
        type: "response.output_item.done",
        item: { type: "reasoning", encrypted_content: "opaque" },
      },
      {
        type: "response.output_item.done",
        item: {
          type: "function_call",
          call_id: "call-list",
          name: "list_dir",
          arguments: '{"path":"."}',
        },
      },
      {
        type: "response.completed",
        response: { status: "completed", output: [] },
      },
    ])) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });
    const streamed: StreamEvent[] = [];

    const message = await openaiCodex.send({
      model: "gpt-codex",
      system: "be useful",
      messages: [{ role: "user", content: [{ kind: "text", text: "inspect" }] }],
      tools: [{
        name: "list_dir",
        description: "List a directory",
        input: { type: "object", properties: { path: { type: "string" } } },
      }],
      maxTokens: 123,
      effort: "high",
      onStream: (event) => streamed.push(event),
    });

    assert.deepEqual(message.content, [
      { kind: "tool_call", id: "call-list", name: "list_dir", input: { path: "." } },
    ]);
    assert.deepEqual(streamed, [{ kind: "tool", name: "list_dir" }]);
  });
});

test("one 401 refreshes the account once and retries with the rotated access token", async (context) => {
  await inStore(async () => {
    await saveAccount("old-access", "old-refresh");
    const previousFetch = globalThis.fetch;
    const authorizations: string[] = [];
    const sessionIds: string[] = [];
    const requestIds: string[] = [];
    const cacheKeys: unknown[] = [];
    let refreshes = 0;
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input);
      if (url.startsWith("https://auth.openai.com/oauth/token")) {
        refreshes++;
        return json({
          access_token: accessToken("new-access"),
          refresh_token: "new-refresh",
          expires_in: 3_600,
          id_token: jwt({ email: "person@example.test" }, "identity"),
        });
      }
      const authorization = new Headers(init?.headers).get("authorization") ?? "";
      const requestHeaders = new Headers(init?.headers);
      authorizations.push(authorization);
      sessionIds.push(requestHeaders.get("session-id") ?? "");
      requestIds.push(requestHeaders.get("x-client-request-id") ?? "");
      cacheKeys.push((JSON.parse(String(init?.body)) as Record<string, unknown>)["prompt_cache_key"]);
      if (authorizations.length === 1) return new Response("unauthorized", { status: 401 });
      return sse({ type: "response.completed", response: { status: "completed", output: [] } });
    }) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });

    await openaiCodex.send({
      model: "gpt-codex",
      system: "be useful",
      messages: [],
      tools: [],
      maxTokens: 100,
      effort: "high",
      identity: {
        conversationId: "22222222-2222-4222-8222-222222222222",
        cacheKey: "jecode-retry-cache",
        purpose: "turn",
      },
    });

    assert.equal(refreshes, 1);
    assert.equal(authorizations[0], "Bearer old-access");
    assert.equal(authorizations[1], `Bearer ${accessToken("new-access")}`);
    assert.deepEqual(sessionIds, [
      "22222222-2222-4222-8222-222222222222",
      "22222222-2222-4222-8222-222222222222",
    ]);
    assert.deepEqual(cacheKeys, ["jecode-retry-cache", "jecode-retry-cache"]);
    assert.match(requestIds[0] ?? "", /^[0-9a-f-]{36}$/u);
    assert.match(requestIds[1] ?? "", /^[0-9a-f-]{36}$/u);
    assert.notEqual(requestIds[0], requestIds[1]);
    assert.equal(openAICodexAccount()?.refreshToken, "new-refresh");
  });
});

test("the ChatGPT provider omits prompt cache routing from compaction", async (context) => {
  await inStore(async () => {
    await saveAccount("send-access", "send-refresh");
    const previousFetch = globalThis.fetch;
    let body: Record<string, unknown> = {};
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      body = JSON.parse(String(init?.body)) as Record<string, unknown>;
      return sse({ type: "response.completed", response: { status: "completed", output: [] } });
    }) as typeof fetch;
    context.after(() => { globalThis.fetch = previousFetch; });

    await openaiCodex.send({
      model: "gpt-codex",
      system: "summarize",
      messages: [],
      tools: [],
      maxTokens: 100,
      effort: "high",
      identity: {
        conversationId: "33333333-3333-4333-8333-333333333333",
        cacheKey: "jecode-compaction-cache",
        purpose: "compaction",
      },
    });

    assert.equal(body["prompt_cache_key"], undefined);
  });
});

async function inStore(body: () => Promise<void>): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-openai-codex-"));
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

async function saveAccount(accessTokenValue: string, refreshToken: string): Promise<void> {
  await updateOpenAICodexAccount(async () => ({
    accessToken: accessTokenValue,
    refreshToken,
    expiresAt: Date.now() + 3_600_000,
    accountId: "account-1",
  }));
}

function accessToken(signature: string): string {
  return jwt({ [CLAIMS]: { chatgpt_account_id: "account-1", chatgpt_plan_type: "plus" } }, signature);
}

function jwt(payload: Record<string, unknown>, signature: string): string {
  const header = Buffer.from(JSON.stringify({ alg: "none" })).toString("base64url");
  const body = Buffer.from(JSON.stringify(payload)).toString("base64url");
  return `${header}.${body}.${signature}`;
}

function json(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function sse(event: unknown): Response {
  return sseEvents([event]);
}

function sseEvents(events: unknown[]): Response {
  return new Response(events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""), {
    status: 200,
    headers: { "content-type": "text/event-stream" },
  });
}
