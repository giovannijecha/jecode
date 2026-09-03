import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import type { Session } from "../src/session.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { appInput } from "../src/tui/app-input.ts";
import { appState } from "../src/tui/app-state.ts";
import { begin } from "../src/tui/activity.ts";
import * as edit from "../src/tui/editor.ts";
import type { Feedback, FeedbackController } from "../src/tui/feedback.ts";
import type { Key } from "../src/tui/keys.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

const key = (name: string, text = "", ctrl = false): Key => ({ name, text, ctrl });

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

function harness(from = provider()): {
  state: ReturnType<typeof appState>;
  feedback: Feedback[];
  commands: string[];
  turns: string[];
  steering: string[];
  scrolls: number[];
  input: ReturnType<typeof appInput>;
  quit(): boolean;
  requestedQuit(): boolean;
  invalidated(): boolean;
} {
  const state = appState();
  const shown: Feedback[] = [];
  const commands: string[] = [];
  const turns: string[] = [];
  const steering: string[] = [];
  const scrolls: number[] = [];
  let quit = false;
  let requestedQuit = false;
  let invalidated = false;
  const feedback: FeedbackController = {
    show(value) {
      state.feedback = value;
      shown.push(value);
    },
    dismiss() {
      state.feedback = undefined;
    },
    close() {},
  };
  const input = appInput({
    session: session(from),
    state,
    feedback,
    actions: {
      command: async (text) => {
        commands.push(text);
      },
      turn: async (text) => {
        turns.push(text);
      },
      steer: (text) => {
        steering.push(text);
        return state.activity?.kind === "turn" ? "queued" : "unavailable";
      },
    },
    live: () => true,
    quit: () => {
      quit = true;
    },
    requestQuit: () => {
      requestedQuit = true;
    },
    scrollBy: (amount) => scrolls.push(amount),
    invalidate: () => {
      invalidated = true;
    },
    transcriptChanged: () => {},
  });

  return {
    state,
    feedback: shown,
    commands,
    turns,
    steering,
    scrolls,
    input,
    quit: () => quit,
    requestedQuit: () => requestedQuit,
    invalidated: () => invalidated,
  };
}

test("a blocked turn keeps the unsent prompt and history untouched", () => {
  const current = harness(provider("ANTHROPIC_API_KEY is not set"));
  current.state.editor = edit.of("keep this prompt");

  current.input.handle(key("enter"));

  assert.equal(current.state.editor.text, "keep this prompt");
  assert.deepEqual(current.state.past, []);
  assert.deepEqual(current.turns, []);
  assert.match(current.feedback[0]?.text ?? "", /needs an API key/);
});

test("submission, completion, and recall preserve the editor draft", () => {
  const current = harness();
  current.state.editor = edit.of("first");
  current.input.handle(key("enter"));
  current.state.editor = edit.of("second");
  current.input.handle(key("enter"));
  current.state.editor = edit.of("draft");

  current.input.handle(key("up"));
  assert.equal(current.state.editor.text, "second");
  current.input.handle(key("up"));
  assert.equal(current.state.editor.text, "first");
  current.input.handle(key("down"));
  current.input.handle(key("down"));
  assert.equal(current.state.editor.text, "draft");

  current.state.editor = edit.of("/he");
  current.input.handle(key("tab"));
  assert.equal(current.state.editor.text, "/help");
  current.input.handle(key("enter"));
  assert.deepEqual(current.commands, ["/help"]);
  assert.deepEqual(current.turns, ["first", "second"]);
});

test("navigation and control keys target the active terminal operation", () => {
  const current = harness();
  current.input.handle({
    name: "pointer",
    text: "",
    ctrl: false,
    pointer: { action: "wheel", button: "none", wheel: "up", col: 0, row: 0 },
  });
  current.input.handle(key("pagedown"));
  current.input.handle(key("l", "", true));
  assert.deepEqual(current.scrolls, [3, -8]);
  assert.equal(current.invalidated(), true);

  current.state.activity = begin("turn", "Working");
  current.input.handle(key("escape"));
  assert.equal(current.state.activity.control.signal.aborted, true);

  current.state.activity = undefined;
  current.input.handle(key("c", "", true));
  assert.equal(current.quit(), true);

  const cleanExit = harness();
  cleanExit.input.handle(key("d", "", true));
  assert.equal(cleanExit.requestedQuit(), true);
});

test("enter queues ordinary guidance during a turn without opening another workflow", () => {
  const current = harness();
  current.state.activity = begin("turn", "Working");
  current.state.editor = edit.of("change the test, not the implementation");

  current.input.handle(key("enter"));

  assert.deepEqual(current.steering, ["change the test, not the implementation"]);
  assert.deepEqual(current.turns, []);
  assert.equal(current.state.editor.text, "");
  assert.deepEqual(current.state.past, ["change the test, not the implementation"]);
});

test("active work keeps slash commands and command-time prompts in the composer", () => {
  const slash = harness();
  slash.state.activity = begin("turn", "Working");
  slash.state.editor = edit.of("/models");
  slash.input.handle(key("enter"));
  assert.equal(slash.state.editor.text, "/models");
  assert.deepEqual(slash.commands, []);
  assert.match(slash.feedback[0]?.text ?? "", /Slash commands.*prompt kept/);

  const command = harness();
  command.state.activity = begin("command", "Running /models");
  command.state.editor = edit.of("keep this");
  command.input.handle(key("enter"));
  assert.equal(command.state.editor.text, "keep this");
  assert.match(command.feedback[0]?.text ?? "", /active command.*prompt kept/);
});

test("ctrl+o can inspect a compacted tool diff before answering an open prompt", () => {
  const current = harness();
  current.state.blocks.push({
    kind: "tool",
    name: "write_file",
    target: "large.txt",
    right: "pending approval",
    tone: "pending",
    body: [{ kind: "add", text: "value", newLine: 1 }],
  });
  current.state.open = {
    picker: {
      title: [],
      options: [{ label: "Yes" }, { label: "No" }],
      index: 0,
    },
    settle() {},
  };

  current.input.handle(key("o", "", true));

  const block = current.state.blocks[0];
  assert.equal(block?.kind === "tool" ? block.expanded : undefined, true);
  assert.ok(current.state.open !== undefined);
});
