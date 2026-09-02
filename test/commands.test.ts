import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { ConversationTree } from "../src/conversation.ts";
import type { Session } from "../src/session.ts";
import type { NoticeBlock } from "../src/tui/blocks.ts";
import type { Picker } from "../src/tui/picker.ts";
import type { Field } from "../src/tui/field.ts";
import type { Host } from "../src/commands.ts";
import { COMMANDS, handleCommand } from "../src/commands.ts";
import { modelsCommand } from "../src/model-command.ts";
import {
  listModels,
  MAX_MODEL_CATALOG_ENTRIES,
  MAX_MODEL_CATALOG_ITEMS,
  MAX_MODEL_ID_CHARS,
} from "../src/providers/catalog.ts";
import { keyFor, reload } from "../src/credentials.ts";
import type { SavedSettings } from "../src/settings.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";
import { builtinTools } from "../src/tools/index.ts";
import { sessionPermissions } from "../src/permissions.ts";

function provider(id: string, models: string[], why?: string): Provider {
  return {
    id,
    defaultModel: models[0] ?? "",
    auth: { kind: "api-key", keyVar: `${id.toUpperCase()}_API_KEY` },
    blocked: () => why,
    models: () => Promise.resolve(models),
    send: (_req: SendRequest): Promise<Message> => Promise.reject(new Error("not called")),
  };
}

function session(from: Provider): Session {
  return {
    config: {
      providerId: from.id,
      model: from.defaultModel,
      reducedMotion: false,
      effort: "high",
      maxTokens: 4096,
      maxSteps: 8,
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider: from,
    model: from.defaultModel,
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}

type Screen = Host & {
  blocks: NoticeBlock[];
  pickers: Picker[];
  fields: Field[];
  helps: number;
  /** What the next field hands back. Unset means the user backed out. */
  typed?: string;
};

/** A host that answers the menus in the order given, and remembers what it saw. */
function host(...answers: (number | undefined)[]): Screen {
  const screen: Screen = {
    blocks: [],
    pickers: [],
    fields: [],
    helps: 0,
    emit: (block) => {
      screen.blocks.push(block);
    },
    choose: (picker) => {
      screen.pickers.push(picker);
      return Promise.resolve(answers.shift());
    },
    type: (field) => {
      screen.fields.push(field);
      return Promise.resolve(screen.typed);
    },
    showHelp: () => {
      screen.helps++;
      return Promise.resolve();
    },
  };
  return screen;
}

function texts(blocks: NoticeBlock[]): string[] {
  return blocks.map((block) => block.text);
}

test("the model menu offers what the provider says it has", async () => {
  const fake = provider("fake", ["big", "small"]);
  const live = session(fake);
  const screen = host(1);

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.deepEqual(
    screen.pickers[0]?.options.map((option) => option.label),
    ["big", "small"],
  );
  assert.deepEqual(screen.pickers[0]?.title, []);
  assert.equal(live.model, "small");
});

test("a direct model choice is offered to the interactive settings store", async () => {
  const live = session(provider("ollama", ["first", "second"]));
  const screen = host(1);
  const saved: Partial<SavedSettings>[] = [];
  screen.saveSettings = (patch) => {
    saved.push(patch);
    return Promise.resolve();
  };

  await modelsCommand(live, screen, {}, [live.provider]);

  assert.equal(live.model, "second");
  assert.equal(saved[0]?.models?.ollama, "second");
});

test("choosing a model reconciles and saves an unsupported effort", async () => {
  const fake = provider("fake", ["first", "second"]);
  fake.efforts = (model) => Promise.resolve(
    model === "second" ? ["low", "medium", "high"] : ["low", "medium", "high", "max"],
  );
  const live = session(fake);
  live.config.effort = "max";
  const screen = host(1);
  const saved: Partial<SavedSettings>[] = [];
  screen.saveSettings = (patch) => {
    saved.push(patch);
    return Promise.resolve();
  };

  await modelsCommand(live, screen, {}, [fake]);

  assert.equal(live.model, "second");
  assert.equal(live.config.effort, "high");
  assert.equal(saved[0]?.effort, "high");
});

test("a direct model choice rolls back when its default cannot be saved", async () => {
  const live = session(provider("ollama", ["first", "second"]));
  const screen = host(1);
  screen.saveSettings = () => Promise.reject(new Error("disk unavailable"));

  await modelsCommand(live, screen, {}, [live.provider]);

  assert.equal(live.model, "first");
  assert.equal(live.config.model, "first");
  assert.match(texts(screen.blocks).join("\n"), /could not save settings · disk unavailable/);
});

test("the model menu opens with the current model selected", async () => {
  const live = session(provider("fake", ["a", "b", "c"]));
  live.model = "c";
  const screen = host();

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.equal(screen.pickers[0]?.index, 2);
});

test("cancelling the model menu leaves the model alone", async () => {
  const live = session(provider("fake", ["a", "b"]));
  const screen = host(undefined);

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.equal(live.model, "a");
});

test("a provider with no key is never asked for its models", async () => {
  const blocked = provider("fake", ["a"], "FAKE_API_KEY is not set");
  let asked = false;
  const live = session({
    ...blocked,
    models: () => {
      asked = true;
      return Promise.resolve([]);
    },
  });
  const screen = host(0);

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.equal(asked, false);
  assert.equal(screen.pickers.length, 0);
  assert.deepEqual(texts(screen.blocks), ["no providers are connected · use /providers"]);
});

test("the model menu aggregates providers and switches provider and model atomically", async () => {
  const first = provider("first", ["shared", "first-only"]);
  const second = provider("second", ["shared", "second-only"]);
  const blocked = provider("blocked", ["hidden"], "BLOCKED_API_KEY is not set");
  const live = session(first);
  const screen = host(3);
  const saved: Partial<SavedSettings>[] = [];
  screen.saveSettings = (patch) => {
    saved.push(patch);
    return Promise.resolve();
  };

  await modelsCommand(live, screen, {}, [first, second, blocked]);

  assert.deepEqual(
    screen.pickers[0]?.options.map((option) => `${option.label}:${option.value}`),
    ["shared:First", "first-only:First", "shared:Second", "second-only:Second"],
  );
  assert.match(screen.pickers[0]?.description ?? "", /Not connected: Blocked/);
  assert.equal(live.provider, second);
  assert.equal(live.model, "second-only");
  assert.equal(live.config.providerId, "second");
  assert.equal(live.config.model, "second-only");
  assert.equal(saved[0]?.provider, "second");
  assert.equal(saved[0]?.models?.second, "second-only");
});

test("one failed catalogue does not hide models from another provider", async () => {
  const good = provider("good", ["usable"]);
  const failed = {
    ...provider("failed", []),
    models: () => Promise.reject(new Error("offline")),
  };
  const live = session(good);
  const screen = host(undefined);

  await modelsCommand(live, screen, { save: false }, [good, failed]);

  assert.deepEqual(screen.pickers[0]?.options.map((option) => option.label), ["usable"]);
  assert.match(screen.pickers[0]?.description ?? "", /Unavailable: Failed/);
  assert.deepEqual(screen.blocks, []);
});

test("model catalogues start concurrently", async () => {
  let resolveFirst!: (models: string[]) => void;
  let resolveSecond!: (models: string[]) => void;
  const firstReady = new Promise<string[]>((resolve) => {
    resolveFirst = resolve;
  });
  const secondReady = new Promise<string[]>((resolve) => {
    resolveSecond = resolve;
  });
  const started: string[] = [];
  const first = {
    ...provider("first", ["one"]),
    models: () => {
      started.push("first");
      return firstReady;
    },
  };
  const second = {
    ...provider("second", ["two"]),
    models: () => {
      started.push("second");
      return secondReady;
    },
  };
  const pending = modelsCommand(session(first), host(undefined), { save: false }, [first, second]);

  await Promise.resolve();
  assert.deepEqual(started, ["first", "second"]);

  resolveFirst(["one"]);
  resolveSecond(["two"]);
  await pending;
});

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

test("the provider menu names every provider and its access state", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();

  await handleCommand("/providers", live, screen);

  const options = screen.pickers[0]?.options ?? [];
  assert.deepEqual(
    options.map((option) => option.label),
    ["Anthropic", "OpenAI API", "ChatGPT", "Ollama"],
  );
  // Either a reason or "ready" — never an empty hint, which would read as a
  // provider that has nothing to say about itself.
  for (const option of options) assert.notEqual(option.value, undefined);
  assert.deepEqual(screen.pickers[0]?.title, []);
  assert.equal(screen.pickers[0]?.description, undefined);
  assert.equal(live.provider.id, "fake");
});

test("provider access management never changes the runtime selection", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(undefined);
  const saved: Partial<SavedSettings>[] = [];
  screen.saveSettings = (patch) => {
    saved.push(patch);
    return Promise.resolve();
  };

  await handleCommand("/providers", live, screen);

  assert.equal(live.provider.id, "fake");
  assert.equal(live.model, "a");
  assert.deepEqual(saved, []);
});

test("ctrl+c inside provider management does not reopen the parent menu", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();
  const control = new AbortController();
  let calls = 0;
  screen.signal = control.signal;
  screen.choose = (picker) => {
    screen.pickers.push(picker);
    calls++;
    if (calls === 1) return Promise.resolve(3); // Ollama
    if (calls === 2) control.abort(new Error("interrupted"));
    return Promise.resolve(undefined);
  };

  await assert.rejects(handleCommand("/providers", live, screen), /interrupted/);

  assert.equal(calls, 2);
});

test("credentials is absent because providers owns access", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();

  assert.equal(COMMANDS.some((command) => command.name === "credentials"), false);
  await handleCommand("/credentials", live, screen);

  assert.match(texts(screen.blocks)[0] ?? "", /unknown command \/credentials/);
});

test("a failure fetching models is reported, not thrown", async () => {
  const live = session({
    ...provider("fake", []),
    models: () => Promise.reject(new Error("network error")),
  });
  const screen = host();

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.deepEqual(texts(screen.blocks), ["Fake: network error"]);
});

test("an unreachable local Ollama catalogue points back to providers", async () => {
  const live = session({
    ...provider("ollama", []),
    location: () => "local",
    models: () => Promise.reject(
      new Error("network error calling http://127.0.0.1:11434/v1/models: fetch failed"),
    ),
  });
  const screen = host();

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.deepEqual(texts(screen.blocks), [
    "Ollama is not reachable on this computer · start Ollama or choose cloud in /providers",
  ]);
});

test("the model request receives the command cancellation signal and retry status", async () => {
  const control = new AbortController();
  const statuses: (string | undefined)[] = [];
  let received: AbortSignal | undefined;
  const live = session({
    ...provider("fake", ["a"]),
    models: (signal, onStatus) => {
      received = signal;
      onStatus?.("Rate limited · retrying in 1s");
      return Promise.resolve(["a"]);
    },
  });
  const screen = host(undefined);
  screen.signal = control.signal;
  screen.status = (status) => statuses.push(status);

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.equal(received, control.signal);
  assert.ok(statuses.some((status) => status?.endsWith("Rate limited · retrying in 1s")));
});

test("cancelling catalogue loading propagates interruption instead of a provider error", async () => {
  const control = new AbortController();
  const statuses: (string | undefined)[] = [];
  const live = session({
    ...provider("fake", ["a"]),
    models: (signal) => new Promise<string[]>((_resolve, reject) => {
      signal?.addEventListener("abort", () => reject(signal.reason), { once: true });
    }),
  });
  const screen = host();
  screen.signal = control.signal;
  screen.status = (status) => statuses.push(status);

  const pending = modelsCommand(live, screen, { save: false }, [live.provider]);
  control.abort(new Error("interrupted"));

  await assert.rejects(pending, /interrupted/);
  assert.deepEqual(screen.blocks, []);
  assert.equal(screen.pickers.length, 0);
  assert.equal(statuses.at(-1), undefined);
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

test("permissions always opens the tool control plane without footer noise", async () => {
  const screen = host(undefined);
  screen.permissions = sessionPermissions(builtinTools(), false);

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.deepEqual(screen.pickers[0]?.options.map((option) => option.label), [
    "read_file",
    "list_dir",
    "find_files",
    "search_text",
    "edit_file",
    "write_file",
    "run_command",
  ]);
  assert.deepEqual(screen.pickers[0]?.title, []);
  assert.equal(screen.pickers[0]?.description, undefined);
  assert.equal(screen.pickers[0]?.visible, 7);
  assert.deepEqual(screen.blocks, []);
});

test("enter on a tool without remembered approvals keeps the control plane open", async () => {
  const screen = host(0, undefined);
  screen.permissions = sessionPermissions(builtinTools(), false);

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(screen.pickers.length, 2);
  assert.deepEqual(screen.pickers[1]?.title, []);
  assert.deepEqual(screen.pickers[1]?.options, screen.pickers[0]?.options);
  assert.deepEqual(screen.blocks, []);
});

test("permissions changes one dangerous tool inline for the session", async () => {
  const screen = host();
  const control = sessionPermissions(builtinTools(), false);
  screen.permissions = control;
  let changed: Picker | undefined;
  screen.choose = (picker) => {
    screen.pickers.push(picker);
    changed = picker.adjust?.(6, 1);
    return Promise.resolve(undefined);
  };

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(control.listTools().find((tool) => tool.name === "run_command")?.mode, "allow");
  assert.equal(changed?.options[6]?.value, "allow");
  assert.equal(changed?.index, 6);
  assert.deepEqual(screen.blocks, []);
});

test("enter reviews and revokes a remembered approval", async () => {
  const screen = host(6, 0, undefined);
  const control = sessionPermissions(builtinTools(), false);
  control.remember({ kind: "tool_call", id: "1", name: "run_command", input: { command: "npm test" } });
  screen.permissions = control;

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.deepEqual(control.listGrants("run_command"), []);
  assert.deepEqual(screen.blocks, []);
});

test("permissions can hide a read-only tool from later turns", async () => {
  const screen = host();
  const control = sessionPermissions(builtinTools(), false);
  screen.permissions = control;
  screen.choose = (picker) => {
    screen.pickers.push(picker);
    picker.adjust?.(0, 1);
    return Promise.resolve(undefined);
  };

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(control.listTools()[0]?.mode, "deny");
  assert.equal(control.availableTools().some((tool) => tool.name === "read_file"), false);
});

test("permissions can revoke every remembered approval for one tool", async () => {
  const screen = host(6, 2, undefined);
  const control = sessionPermissions(builtinTools(), false);
  control.remember({ kind: "tool_call", id: "1", name: "run_command", input: { command: "npm test" } });
  control.remember({ kind: "tool_call", id: "2", name: "run_command", input: { command: "npm run check" } });
  screen.permissions = control;

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.deepEqual(control.listGrants("run_command"), []);
});

test("permissions keeps a launch-time auto-approve override locked inline", async () => {
  const screen = host(6, undefined);
  screen.permissions = sessionPermissions(builtinTools(), true);

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(screen.pickers[0]?.options[6]?.value, "allow · locked");
  assert.equal(screen.pickers[0]?.options[6]?.adjustable, false);
  assert.match(texts(screen.blocks)[0] ?? "", /restart without --auto-approve/);
});

test("cancelling provider access leaves the runtime selection intact", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(1, undefined, undefined); // OpenAI API, back, providers back
  const before = process.env["OPENAI_API_KEY"];
  delete process.env["OPENAI_API_KEY"];
  try {
    await handleCommand("/providers", live, screen);
    assert.equal(live.provider.id, "fake");
    assert.equal(live.model, "a");
  } finally {
    if (before !== undefined) process.env["OPENAI_API_KEY"] = before;
    reload();
  }
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

test("a model list keeps the ids and drops everything else", async () => {
  const body = { data: [{ id: "one" }, { id: 7 }, null, 4, {}, { id: "two" }] };
  const original = globalThis.fetch;
  globalThis.fetch = (() =>
    Promise.resolve(new Response(JSON.stringify(body), { status: 200 }))) as typeof fetch;

  try {
    assert.deepEqual(await listModels("https://example.test/v1/models", {}), ["one", "two"]);
  } finally {
    globalThis.fetch = original;
  }
});

test("a model catalogue is deduplicated and bounded", async () => {
  const body = {
    data: [
      { id: "same" },
      { id: "same" },
      { id: "x".repeat(MAX_MODEL_ID_CHARS + 1) },
      ...Array.from(
        { length: MAX_MODEL_CATALOG_ENTRIES + 10 },
        (_, index) => ({ id: `model-${index}` }),
      ),
    ],
  };
  const original = globalThis.fetch;
  globalThis.fetch = (() =>
    Promise.resolve(new Response(JSON.stringify(body), { status: 200 }))) as typeof fetch;

  try {
    const models = await listModels("https://example.test/v1/models", {});
    assert.equal(models.length, MAX_MODEL_CATALOG_ENTRIES);
    assert.equal(models[0], "same");
    assert.equal(new Set(models).size, models.length);
    assert.equal(models.some((id) => id.length > MAX_MODEL_ID_CHARS), false);
  } finally {
    globalThis.fetch = original;
  }
});

test("a model catalogue stops inspecting an oversized invalid list", async () => {
  const body = {
    data: [
      ...Array.from({ length: MAX_MODEL_CATALOG_ITEMS }, () => ({})),
      { id: "outside-the-scan-budget" },
    ],
  };
  const original = globalThis.fetch;
  globalThis.fetch = (() =>
    Promise.resolve(new Response(JSON.stringify(body), { status: 200 }))) as typeof fetch;

  try {
    assert.deepEqual(await listModels("https://example.test/v1/models", {}), []);
  } finally {
    globalThis.fetch = original;
  }
});

test("provider access can collect a missing API key, masked", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(1, 0, 0, undefined); // OpenAI API, add, session, back
  screen.typed = "typed-key";

  const before = process.env["OPENAI_API_KEY"];
  delete process.env["OPENAI_API_KEY"];
  try {
    await handleCommand("/providers", live, screen);

    assert.equal(screen.fields.length, 1, "no key was asked for");
    assert.equal(screen.fields[0]?.secret, true, "the field would show the key");
    assert.equal(keyFor("OPENAI_API_KEY"), "typed-key");
  } finally {
    if (before !== undefined) process.env["OPENAI_API_KEY"] = before;
    reload();
  }
});

test("the key never reaches the transcript", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(1, 0, 0, undefined);
  screen.typed = "fixture-credential-value";

  const before = process.env["OPENAI_API_KEY"];
  delete process.env["OPENAI_API_KEY"];
  try {
    await handleCommand("/providers", live, screen);
    for (const said of texts(screen.blocks)) {
      assert.ok(!said.includes("fixture-credential-value"), `the key was printed: ${said}`);
    }
  } finally {
    if (before !== undefined) process.env["OPENAI_API_KEY"] = before;
    reload();
  }
});

test("discarding a typed key keeps it out of the session", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(1, 0, 2, undefined); // OpenAI API, add, discard, back
  screen.typed = "throwaway";

  const before = process.env["OPENAI_API_KEY"];
  delete process.env["OPENAI_API_KEY"];
  try {
    await handleCommand("/providers", live, screen);
    assert.equal(keyFor("OPENAI_API_KEY"), undefined);
  } finally {
    if (before !== undefined) process.env["OPENAI_API_KEY"] = before;
    reload();
  }
});

test("a provider that can already run is not asked for a key", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(0, undefined);

  const before = process.env["ANTHROPIC_API_KEY"];
  process.env["ANTHROPIC_API_KEY"] = "already-there";
  try {
    await handleCommand("/providers", live, screen);
    assert.equal(screen.fields.length, 0);
  } finally {
    if (before === undefined) delete process.env["ANTHROPIC_API_KEY"];
    else process.env["ANTHROPIC_API_KEY"] = before;
    reload();
  }
});
