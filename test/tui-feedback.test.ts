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
      compactionPercent: 85,
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
    { status: "Responding · 12s", feedback: undefined, readiness: undefined, unseen: 0 },
    STEEL,
  );

  assert.equal(
    status.map((segment) => segment.text).join(""),
    "Responding · 12s · esc to interrupt",
  );
});

test("active model work exposes steering availability and queued guidance", () => {
  const available = renderStatus(
    {
      status: "Thinking · 3s",
      steering: 0,
      feedback: undefined,
      readiness: undefined,
      unseen: 0,
    },
    STEEL,
  );
  const queued = renderStatus(
    {
      status: "Running edit_file · 8s",
      steering: 2,
      feedback: undefined,
      readiness: undefined,
      unseen: 0,
    },
    STEEL,
  );

  assert.equal(
    available.map((segment) => segment.text).join(""),
    "Thinking · 3s · enter to steer · esc to interrupt",
  );
  assert.equal(
    queued.map((segment) => segment.text).join(""),
    "2 queued · Running edit_file · 8s · esc to interrupt",
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

test("the footer projection exposes the exact provider route without secondary noise", () => {
  assert.deepEqual(footerInfo(session(provider()), "~/Codex/jecode"), {
    workspace: "~/Codex/jecode",
    provider: "Anthropic API",
    model: "claude-sonnet-5",
    effort: "high",
  });
});

test("turn blockers use user-facing copy and remain until the next key", () => {
  const needsKey = turnBlocker(session(provider("ANTHROPIC_API_KEY is not set")));
  assert.deepEqual(needsKey, {
    text: "Anthropic API needs an API key · /providers",
    tone: "warn",
  });

  const needsModel = turnBlocker(session(provider(), ""));
  assert.deepEqual(needsModel, {
    text: "Anthropic API needs a model · /models",
    tone: "warn",
  });
  assert.equal(turnBlocker(session(provider())), undefined);
});

test("the OpenAI Account provider asks for sign-in instead of an API key", () => {
  const codex: Provider = {
    ...provider(),
    id: "openai-codex",
    defaultModel: "",
    auth: { kind: "oauth", account: "openai-codex", label: "OpenAI Account" },
    blocked: () => "OpenAI Account is not connected",
  };

  assert.deepEqual(turnBlocker(session(codex)), {
    text: "OpenAI Account needs sign-in · /providers",
    tone: "warn",
  });
  const failure = turnFailure(session(codex, "gpt-codex"), new Error("401 unauthorized"), false);
  assert.match(failure.text, /reconnect OpenAI Account in \/providers/);
  assert.equal(footerInfo(session(codex)).provider, "OpenAI Account");
});

test("a real authentication failure remains one actionable transcript notice", () => {
  const error = Object.assign(new Error("401 unauthorized"), {
    body: '{"error":{"message":"invalid account"}}',
  });
  const failure = turnFailure(session(provider()), error, false);
  assert.equal(failure.kind, "notice");
  assert.equal(failure.tone, "error");
  assert.match(failure.text, /^Anthropic API: authentication failed · /);
  assert.match(failure.text, /access|environment/);
});

test("an Ollama network failure becomes one actionable transcript notice", () => {
  const ollama: Provider = { ...provider(), id: "ollama" };
  const failure = turnFailure(
    session(ollama),
    new Error("network error calling https://ollama.com/v1/chat/completions: fetch failed"),
    false,
  );

  assert.equal(failure.text, "Ollama API: network request failed · check the connection and retry");
  assert.equal(failure.tone, "error");
});
