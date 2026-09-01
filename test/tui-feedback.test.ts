import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import type { Session } from "../src/session.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { commandFeedback, feedbackController, turnBlocker } from "../src/tui/feedback.ts";
import { renderStatus } from "../src/tui/components/status.ts";
import { footerInfo, turnFailure } from "../src/tui/session-view.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

function provider(blocked?: string): Provider {
  return {
    id: "anthropic",
    defaultModel: "claude-sonnet-5",
    auth: { kind: "api-key", keyVar: "ANTHROPIC_API_KEY" },
    blocked: () => blocked,
    models: () => Promise.resolve([]),
    send: (_request: SendRequest): Promise<Message> => Promise.reject(new Error("not called")),
  };
}

function session(from: Provider, model = from.defaultModel): Session {
  return {
    config: {
      providerId: from.id,
      model,
      reducedMotion: false,
      effort: "high",
      maxTokens: 4096,
      maxSteps: 8,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider: from,
    model,
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}

test("command notices become expiring footer feedback", () => {
  const feedback = commandFeedback({ kind: "notice", text: "credential saved", tone: "info" });
  assert.equal(feedback?.text, "credential saved");
  assert.ok((feedback?.timeoutMs ?? 0) > 0);
});

test("informational feedback has no decorative marker", () => {
  const status = renderStatus(
    { status: undefined, feedback: { text: "new session", tone: "info" }, readiness: undefined, unseen: 0 },
    STEEL,
  );

  assert.equal(status.map((segment) => segment.text).join(""), "new session");
});

test("active work exposes only state, elapsed time, and the interrupt hint", () => {
  const status = renderStatus(
    { status: "Writing · 12s", feedback: undefined, readiness: undefined, unseen: 0 },
    STEEL,
  );

  assert.equal(
    status.map((segment) => segment.text).join(""),
    "Writing · 12s · esc to interrupt",
  );
});

test("warnings and errors keep their priority markers", () => {
  const warning = renderStatus(
    { status: undefined, feedback: { text: "check settings", tone: "warn" }, readiness: undefined, unseen: 0 },
    STEEL,
  );
  const error = renderStatus(
    { status: undefined, feedback: { text: "request failed", tone: "error" }, readiness: undefined, unseen: 0 },
    STEEL,
  );

  assert.equal(warning.map((segment) => segment.text).join(""), "! check settings");
  assert.equal(error.map((segment) => segment.text).join(""), "× request failed");
});

test("the feedback channel replaces its message instead of accumulating copies", () => {
  const seen: (string | undefined)[] = [];
  const channel = feedbackController((feedback) => seen.push(feedback?.text));
  channel.show({ text: "first", tone: "info" });
  channel.show({ text: "second", tone: "warn" });
  channel.dismiss();
  assert.deepEqual(seen, ["first", "second", undefined]);
  channel.close();
});

test("the footer projection omits provider, usage, location, and ready noise", () => {
  assert.deepEqual(footerInfo(session(provider()), "~/Codex/jecode"), {
    workspace: "~/Codex/jecode",
    model: "claude-sonnet-5",
    effort: "high",
  });
});

test("turn blockers use user-facing copy and remain until the next key", () => {
  const needsKey = turnBlocker(session(provider("ANTHROPIC_API_KEY is not set")));
  assert.deepEqual(needsKey, {
    text: "Anthropic needs an API key · /providers",
    tone: "warn",
  });

  const needsModel = turnBlocker(session(provider(), ""));
  assert.deepEqual(needsModel, {
    text: "Anthropic needs a model · /models",
    tone: "warn",
  });
  assert.equal(turnBlocker(session(provider())), undefined);
});

test("the ChatGPT provider asks for sign-in instead of an API key", () => {
  const codex: Provider = {
    ...provider(),
    id: "openai-codex",
    defaultModel: "",
    auth: { kind: "oauth", account: "openai-codex", label: "ChatGPT" },
    blocked: () => "ChatGPT account is not connected",
  };

  assert.deepEqual(turnBlocker(session(codex)), {
    text: "ChatGPT needs sign-in · /providers",
    tone: "warn",
  });
  const failure = turnFailure(session(codex, "gpt-codex"), new Error("401 unauthorized"), false);
  assert.match(failure.text, /reconnect ChatGPT in \/providers/);
});

test("a real authentication failure remains one actionable transcript notice", () => {
  const failure = turnFailure(session(provider()), new Error("401 unauthorized"), false);
  assert.equal(failure.kind, "notice");
  assert.equal(failure.tone, "error");
  assert.match(failure.text, /^401 unauthorized · /);
  assert.match(failure.text, /access|environment/);
});

test("an Ollama network failure becomes one actionable transcript notice", () => {
  const ollama: Provider = { ...provider(), id: "ollama", location: () => "cloud" };
  const failure = turnFailure(
    session(ollama),
    new Error("network error calling https://ollama.com/v1/chat/completions: fetch failed"),
    false,
  );

  assert.equal(failure.text, "Ollama is not reachable · check its connection in /providers");
  assert.equal(failure.tone, "error");
});
