import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import type { Session } from "../src/session.ts";
import type { Block } from "../src/tui/blocks.ts";
import type { Picker } from "../src/tui/picker.ts";
import type { Field } from "../src/tui/field.ts";
import type { Host } from "../src/commands.ts";
import { handleCommand } from "../src/commands.ts";
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

function provider(id: string, models: string[], why?: string): Provider {
  return {
    id,
    defaultModel: models[0] ?? "",
    keyVar: `${id.toUpperCase()}_API_KEY`,
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
      root: process.cwd(),
      autoApprove: false,
    },
    provider: from,
    model: from.defaultModel,
    palette: STEEL,
    tools: [],
    system: "",
    history: [],
    usage: emptyUsage(),
  };
}

type Screen = Host & {
  blocks: Block[];
  pickers: Picker[];
  fields: Field[];
  /** What the next field hands back. Unset means the user backed out. */
  typed?: string;
};

/** A host that answers the menus in the order given, and remembers what it saw. */
function host(...answers: (number | undefined)[]): Screen {
  const screen: Screen = {
    blocks: [],
    pickers: [],
    fields: [],
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
  };
  return screen;
}

function texts(blocks: Block[]): string[] {
  return blocks.map((block) => ("text" in block ? block.text : block.kind));
}

test("the model menu offers what the provider says it has", async () => {
  const fake = provider("fake", ["big", "small"]);
  const live = session(fake);
  const screen = host(1);

  await handleCommand("/models", live, screen);

  assert.deepEqual(
    screen.pickers[0]?.options.map((option) => option.label),
    ["big", "small"],
  );
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

  await handleCommand("/models", live, screen);

  assert.equal(live.model, "second");
  assert.equal(saved[0]?.models?.ollama, "second");
});

test("a direct model choice rolls back when its default cannot be saved", async () => {
  const live = session(provider("ollama", ["first", "second"]));
  const screen = host(1);
  screen.saveSettings = () => Promise.reject(new Error("disk unavailable"));

  await handleCommand("/models", live, screen);

  assert.equal(live.model, "first");
  assert.equal(live.config.model, "first");
  assert.match(texts(screen.blocks).join("\n"), /could not save settings · disk unavailable/);
});

test("the model menu opens with the current model selected", async () => {
  const live = session(provider("fake", ["a", "b", "c"]));
  live.model = "c";
  const screen = host();

  await handleCommand("/models", live, screen);

  assert.equal(screen.pickers[0]?.index, 2);
});

test("cancelling the model menu leaves the model alone", async () => {
  const live = session(provider("fake", ["a", "b"]));
  const screen = host(undefined);

  await handleCommand("/models", live, screen);

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

  await handleCommand("/models", live, screen);

  assert.equal(asked, false);
  assert.equal(screen.pickers.length, 0);
  assert.deepEqual(texts(screen.blocks), ["Fake still needs an API key"]);
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
  const blocks: Block[] = [];

  await handleCommand("/models", live, { emit: (block) => blocks.push(block) });

  assert.equal(asked, false);
  assert.equal(blocks.length, 1);
});

test("the provider menu names every provider and what stops it", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();

  await handleCommand("/providers", live, screen);

  const options = screen.pickers[0]?.options ?? [];
  assert.deepEqual(
    options.map((option) => option.label),
    ["anthropic", "openai", "ollama"],
  );
  // Either a reason or "ready" — never an empty hint, which would read as a
  // provider that has nothing to say about itself.
  for (const option of options) assert.notEqual(option.hint, undefined);
});

test("a direct provider choice is offered to the interactive settings store", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(2);
  const saved: Partial<SavedSettings>[] = [];
  screen.saveSettings = (patch) => {
    saved.push(patch);
    return Promise.resolve();
  };
  const beforeHost = process.env["OLLAMA_HOST"];
  process.env["OLLAMA_HOST"] = "http://127.0.0.1:11434";

  try {
    await handleCommand("/providers", live, screen);
    assert.equal(live.provider.id, "ollama");
    assert.equal(saved[0]?.provider, "ollama");
  } finally {
    if (beforeHost === undefined) delete process.env["OLLAMA_HOST"];
    else process.env["OLLAMA_HOST"] = beforeHost;
  }
});

test("a direct provider choice rolls back when its default cannot be saved", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(2);
  screen.saveSettings = () => Promise.reject(new Error("disk unavailable"));
  const beforeHost = process.env["OLLAMA_HOST"];
  process.env["OLLAMA_HOST"] = "http://127.0.0.1:11434";

  try {
    await handleCommand("/providers", live, screen);
    assert.equal(live.provider.id, "fake");
    assert.equal(live.model, "a");
    assert.equal(live.config.providerId, "fake");
    assert.equal(live.config.model, "a");
    assert.match(texts(screen.blocks).join("\n"), /could not save settings · disk unavailable/);
  } finally {
    if (beforeHost === undefined) delete process.env["OLLAMA_HOST"];
    else process.env["OLLAMA_HOST"] = beforeHost;
  }
});

test("a failure fetching models is reported, not thrown", async () => {
  const live = session({
    ...provider("fake", []),
    models: () => Promise.reject(new Error("network error")),
  });
  const screen = host(0);

  await handleCommand("/models", live, screen);

  assert.deepEqual(texts(screen.blocks), ["fake: network error"]);
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

  await handleCommand("/models", live, screen);

  assert.equal(received, control.signal);
  assert.ok(statuses.includes("Rate limited · retrying in 1s"));
});

test("new clears conversation usage and screen-local state", async () => {
  const live = session(provider("fake", ["a"]));
  live.history.push({ role: "user", content: [{ kind: "text", text: "old" }] });
  live.usage.requests = 2;
  live.usage.inputTokens = 123;
  let reset = false;
  const screen = host();
  screen.reset = () => {
    reset = true;
  };

  await handleCommand("/new", live, screen);

  assert.equal(reset, true);
  assert.deepEqual(live.history, []);
  assert.equal(live.usage.requests, 0);
  assert.equal(live.usage.inputTokens, 0);
  assert.match(texts(screen.blocks)[0] ?? "", /approvals cleared/);
});

test("usage reports cumulative tokens and the latest context", async () => {
  const live = session(provider("fake", ["a"]));
  Object.assign(live.usage, {
    requests: 3,
    lastInputTokens: 1_250,
    inputTokens: 12_000,
    outputTokens: 900,
    cachedInputTokens: 4_000,
  });
  const screen = host();

  await handleCommand("/usage", live, screen);

  const block = screen.blocks[0];
  assert.equal(block?.kind, "list");
  const report = block?.kind === "list" ? block.items.map((item) => item.text).join("\n") : "";
  assert.match(report, /requests\s+3/);
  assert.match(report, /latest context\s+1\.3k/);
  assert.match(report, /input\s+12k/);
  assert.match(report, /cached input\s+4\.0k/);
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
  assert.match(texts(screen.blocks)[0] ?? "", /jecode-transcript-20260829T123456Z\.md/);
});

test("permissions revokes one selected session grant", async () => {
  const screen = host(1);
  const revoked: (string | undefined)[] = [];
  screen.permissions = () => [
    { key: "one", label: "file changes · a.ts" },
    { key: "two", label: "command · npm test" },
  ];
  screen.revokePermission = (key) => revoked.push(key);

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.deepEqual(revoked, ["two"]);
  assert.match(texts(screen.blocks)[0] ?? "", /command · npm test/);
});

test("cancelling setup for a blocked provider leaves the previous provider intact", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(1); // openai, then cancel the secret field
  const before = process.env["OPENAI_API_KEY"];
  delete process.env["OPENAI_API_KEY"];
  try {
    await handleCommand("/providers", live, screen);
    assert.equal(live.provider.id, "fake");
    assert.match(texts(screen.blocks)[0] ?? "", /provider unchanged/);
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

test("picking a provider that cannot run asks for its key, masked", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(1, 0); // openai, then "just this session"
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
  const screen = host(1, 0);
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
  const screen = host(1, 2); // openai, then "discard it"
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
  const screen = host(0, 0);

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
