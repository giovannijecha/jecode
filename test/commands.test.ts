import { test } from "node:test";
import assert from "node:assert/strict";
import type { NoticeBlock } from "../src/tui/blocks.ts";
import { COMMANDS, handleCommand } from "../src/commands.ts";
import { modelsCommand } from "../src/model-command.ts";
import { provider, session, host, texts } from "../dev/test-support/commands.ts";

test("a menu command without a screen says so, and asks nothing", async () => {
  let asked = false;
  const live = session({
    ...provider("fake", ["a"]),
    models: () => {
      asked = true;
      return Promise.resolve(["a"]);
    },
  });
  const blocks: NoticeBlock[] = [];

  await modelsCommand(live, { emit: (block) => blocks.push(block) }, { save: false }, [live.provider]);

  assert.equal(asked, false);
  assert.equal(blocks.length, 1);
});

test("credentials is absent because providers owns access", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();

  assert.equal(COMMANDS.some((command) => command.name === "credentials"), false);
  await handleCommand("/credentials", live, screen);

  assert.match(texts(screen.blocks)[0] ?? "", /unknown command \/credentials/);
});

test("new clears conversation usage and screen-local state", async () => {
  const live = session(provider("fake", ["a"]));
  live.conversation = live.conversation.commit({
    parentId: 0,
    createdAt: new Date(0).toISOString(),
    identity: { providerId: "fake", model: "a", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "old" }] },
      { role: "assistant", content: [{ kind: "text", text: "answer" }] },
    ],
    blocks: [],
  }, "completed");
  live.usage.requests = 2;
  live.usage.inputTokens = 123;
  let reset = false;
  const screen = host();
  screen.reset = () => {
    reset = true;
  };

  await handleCommand("/new", live, screen);

  assert.equal(reset, true);
  assert.deepEqual(live.conversation.history, []);
  assert.equal(live.usage.requests, 0);
  assert.equal(live.usage.inputTokens, 0);
  assert.deepEqual(texts(screen.blocks), ["new session"]);
});

test("timeline announces only a changed branch point", async () => {
  const screen = host();
  screen.timeline = () => Promise.resolve("selected");

  await handleCommand("/timeline", session(provider("fake", ["a"])), screen);

  assert.deepEqual(texts(screen.blocks), ["branch point selected · send a message to continue"]);

  screen.blocks.length = 0;
  screen.timeline = () => Promise.resolve("unchanged");
  await handleCommand("/timeline", session(provider("fake", ["a"])), screen);
  assert.deepEqual(screen.blocks, []);
});

test("timeline stays outside batch surfaces", async () => {
  const screen = host();
  screen.timeline = undefined;

  await handleCommand("/timeline", session(provider("fake", ["a"])), screen);

  assert.deepEqual(texts(screen.blocks), ["timeline needs the interactive screen"]);
});

test("compact reports success, stays silent on no-op, and guards a pending branch", async () => {
  const screen = host();
  screen.compact = () => Promise.resolve("compacted");

  await handleCommand("/compact", session(provider("fake", ["a"])), screen);
  assert.deepEqual(texts(screen.blocks), ["context compacted"]);

  screen.blocks.length = 0;
  screen.compact = () => Promise.resolve("unchanged");
  await handleCommand("/compact", session(provider("fake", ["a"])), screen);
  assert.deepEqual(screen.blocks, []);

  screen.compact = () => Promise.resolve("branch-pending");
  await handleCommand("/compact", session(provider("fake", ["a"])), screen);
  assert.deepEqual(texts(screen.blocks), ["send a message on this branch before compacting"]);
});

test("help opens a temporary surface without emitting transcript content", async () => {
  const screen = host();

  await handleCommand("/help", session(provider("fake", ["a"])), screen);

  assert.equal(screen.helps, 1);
  assert.deepEqual(screen.blocks, []);
});

test("usage remains internal and is absent from the command surface", async () => {
  const screen = host();

  assert.equal(COMMANDS.some((command) => command.name === "usage"), false);
  await handleCommand("/usage", session(provider("fake", ["a"])), screen);

  assert.match(texts(screen.blocks)[0] ?? "", /unknown command \/usage/);
});

test("setup is absent because settings and direct provider commands own configuration", async () => {
  const screen = host();

  assert.equal(COMMANDS.some((command) => command.name === "setup"), false);
  await handleCommand("/setup", session(provider("fake", ["a"])), screen);

  assert.match(texts(screen.blocks)[0] ?? "", /unknown command \/setup/);
});

test("export saves immediately to its automatic destination", async () => {
  const screen = host();
  let exports = 0;
  screen.exportTranscript = () => {
    exports++;
    return Promise.resolve("jecode-transcript-20260829T123456Z.md");
  };

  await handleCommand("/export", session(provider("fake", ["a"])), screen);

  assert.equal(exports, 1);
  assert.equal(screen.pickers.length, 0);
  assert.equal(screen.fields.length, 0);
  assert.deepEqual(texts(screen.blocks), ["saved · jecode-transcript-20260829T123456Z.md"]);
});

test("an unknown command is a notice, not a throw", async () => {
  const screen = host();
  const outcome = await handleCommand("/nope", session(provider("fake", ["a"])), screen);

  assert.equal(outcome, "handled");
  assert.match(texts(screen.blocks)[0] ?? "", /unknown command/);
});

test("exit returns the exit outcome without mutating the session", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();
  assert.equal(await handleCommand("/exit", live, screen), "exit");
  assert.equal(screen.blocks.length, 0);
});
