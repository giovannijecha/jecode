import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { handleCommand } from "../src/commands.ts";
import { keyFor, reload } from "../src/credentials.ts";
import type { SavedSettings } from "../src/settings.ts";
import { provider, session, host, texts } from "../dev/test-support/commands.ts";

async function withIsolatedOpenAICredentials(body: () => Promise<void>): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-command-credentials-"));
  const before = {
    home: process.env["JECODE_HOME"],
    key: process.env["OPENAI_API_KEY"],
  };
  process.env["JECODE_HOME"] = directory;
  delete process.env["OPENAI_API_KEY"];
  reload();
  try {
    await body();
  } finally {
    if (before.home === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before.home;
    if (before.key === undefined) delete process.env["OPENAI_API_KEY"];
    else process.env["OPENAI_API_KEY"] = before.key;
    reload();
    await rm(directory, { recursive: true, force: true });
  }
}

test("the provider menu separates account and API access", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();

  await handleCommand("/providers", live, screen);

  const options = screen.pickers[0]?.options ?? [];
  assert.deepEqual(options.map((option) => option.label), ["Account", "API"]);
  assert.deepEqual(options.map((option) => option.value), ["1 provider", "3 providers"]);
  assert.deepEqual(screen.pickers[0]?.title, []);
  assert.equal(
    screen.pickers[0]?.description,
    "Current route: Fake · access changes do not switch it; use /models",
  );
  assert.equal(live.provider.id, "fake");
});

test("the provider menu marks the active API route before opening a connection", async () => {
  const live = session(provider("openai", ["gpt-fixture"]));
  const screen = host();

  await handleCommand("/providers", live, screen);

  assert.deepEqual(
    screen.pickers[0]?.options.map((option) => option.value),
    ["1 provider", "3 providers · current OpenAI API · billed usage"],
  );
  assert.match(screen.pickers[0]?.description ?? "", /^Current route: OpenAI API · billed usage/);
});

test("the account provider menu contains account-backed authentication", async () => {
  const screen = host(0, undefined, undefined);

  await handleCommand("/providers", session(provider("fake", ["a"])), screen);

  assert.deepEqual(
    screen.pickers[1]?.options.map((option) => option.label),
    ["OpenAI Account"],
  );
  assert.notEqual(screen.pickers[1]?.options[0]?.value, undefined);
  assert.deepEqual(screen.pickers[2]?.options.map((option) => option.label), ["Account", "API"]);
});

test("the API provider menu uses consistent API labels", async () => {
  const screen = host(1, undefined, undefined);

  await handleCommand("/providers", session(provider("fake", ["a"])), screen);

  assert.deepEqual(
    screen.pickers[1]?.options.map((option) => option.label),
    ["Anthropic API", "OpenAI API", "Ollama API"],
  );
  for (const option of screen.pickers[1]?.options ?? []) assert.notEqual(option.value, undefined);
  assert.deepEqual(screen.pickers[2]?.options.map((option) => option.label), ["Account", "API"]);
});

test("provider access management never changes the runtime selection", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(0, 0, undefined, undefined, undefined);
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
    if (calls === 1) return Promise.resolve(1); // API
    if (calls === 2) return Promise.resolve(2); // Ollama
    if (calls === 3) control.abort(new Error("interrupted"));
    return Promise.resolve(undefined);
  };

  await assert.rejects(handleCommand("/providers", live, screen), /interrupted/);

  assert.equal(calls, 3);
});

test("ctrl+c inside a provider category does not reopen the root menu", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host();
  const control = new AbortController();
  let calls = 0;
  screen.signal = control.signal;
  screen.choose = (picker) => {
    screen.pickers.push(picker);
    calls++;
    if (calls === 1) return Promise.resolve(1); // API
    control.abort(new Error("interrupted"));
    return Promise.resolve(undefined);
  };

  await assert.rejects(handleCommand("/providers", live, screen), /interrupted/);

  assert.equal(calls, 2);
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

test("provider access can collect a missing API key, masked", async () => {
  await withIsolatedOpenAICredentials(async () => {
    const live = session(provider("fake", ["a"]));
    const screen = host(1, 1, 0, 0, undefined, undefined); // API, OpenAI, add, session, back
    screen.typed = "typed-key";

    await handleCommand("/providers", live, screen);

    assert.equal(screen.fields.length, 1, "no key was asked for");
    assert.equal(screen.fields[0]?.secret, true, "the field would show the key");
    assert.equal(keyFor("OPENAI_API_KEY"), "typed-key");
  });
});

test("the key never reaches the transcript", async () => {
  await withIsolatedOpenAICredentials(async () => {
    const live = session(provider("fake", ["a"]));
    const screen = host(1, 1, 0, 0, undefined, undefined);
    screen.typed = "fixture-credential-value";

    await handleCommand("/providers", live, screen);
    for (const said of texts(screen.blocks)) {
      assert.ok(!said.includes("fixture-credential-value"), `the key was printed: ${said}`);
    }
  });
});

test("discarding a typed key keeps it out of the session", async () => {
  await withIsolatedOpenAICredentials(async () => {
    const live = session(provider("fake", ["a"]));
    const screen = host(1, 1, 0, 2, undefined, undefined); // API, OpenAI, add, discard, back
    screen.typed = "throwaway";

    await handleCommand("/providers", live, screen);
    assert.equal(keyFor("OPENAI_API_KEY"), undefined);
  });
});

test("a provider that can already run is not asked for a key", async () => {
  const live = session(provider("fake", ["a"]));
  const screen = host(1, 0, undefined, undefined);

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
