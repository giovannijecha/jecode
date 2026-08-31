import { test } from "node:test";
import assert from "node:assert/strict";
import type { Session } from "../src/session.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { commandFeedback, feedbackController, turnBlocker } from "../src/tui/feedback.ts";
import { footerInfo, turnFailure } from "../src/tui/session-view.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

function provider(blocked?: string): Provider {
  return {
    id: "anthropic",
    defaultModel: "claude-sonnet-5",
    keyVar: "ANTHROPIC_API_KEY",
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
    },
    provider: from,
    model,
    palette: STEEL,
    tools: [],
    system: "",
    history: [],
    usage: emptyUsage(),
  };
}

test("command notices become expiring footer feedback", () => {
  const feedback = commandFeedback({ kind: "notice", text: "credential saved", tone: "info" });
  assert.equal(feedback?.text, "credential saved");
  assert.ok((feedback?.timeoutMs ?? 0) > 0);
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
    text: "Anthropic needs an API key · /settings",
    tone: "warn",
  });

  const needsModel = turnBlocker(session(provider(), ""));
  assert.deepEqual(needsModel, {
    text: "Anthropic needs a model · /models",
    tone: "warn",
  });
  assert.equal(turnBlocker(session(provider())), undefined);
});

test("a real authentication failure remains one actionable transcript notice", () => {
  const failure = turnFailure(session(provider()), new Error("401 unauthorized"), false);
  assert.equal(failure.kind, "notice");
  assert.equal(failure.tone, "error");
  assert.match(failure.text, /^401 unauthorized · /);
  assert.match(failure.text, /credentials|environment/);
});

test("an Ollama network failure becomes one actionable transcript notice", () => {
  const ollama: Provider = { ...provider(), id: "ollama", location: () => "cloud" };
  const failure = turnFailure(
    session(ollama),
    new Error("network error calling https://ollama.com/v1/chat/completions: fetch failed"),
    false,
  );

  assert.equal(failure.text, "Ollama is not reachable · check its connection in /settings");
  assert.equal(failure.tone, "error");
});
