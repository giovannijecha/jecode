import { test } from "node:test";
import assert from "node:assert/strict";
import type { Provider } from "../src/types.ts";
import { modelsCommand } from "../src/model-command.ts";
import type { SavedSettings } from "../src/settings.ts";
import { provider, session, host, texts } from "../dev/test-support/commands.ts";

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

test("the model menu names every API route and distinguishes OpenAI Account usage", async () => {
  const api = provider("openai", ["shared-model"]);
  const account: Provider = {
    ...provider("openai-codex", ["shared-model"]),
    auth: { kind: "oauth", account: "openai-codex", label: "OpenAI Account" },
  };
  const screen = host(undefined);

  await modelsCommand(session(api), screen, { save: false }, [
    provider("anthropic", ["claude-model"]), api, account, provider("ollama", ["cloud-model"]),
  ]);

  assert.deepEqual(
    screen.pickers[0]?.options.map((option) => `${option.label}:${option.value}`),
    [
      "claude-model:Anthropic API", "shared-model:OpenAI API · billed usage",
      "shared-model:OpenAI Account", "cloud-model:Ollama API",
    ],
  );
  assert.match(screen.pickers[0]?.description ?? "", /API and account usage stay separate/);
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

test("a failure fetching models is reported, not thrown", async () => {
  const live = session({
    ...provider("fake", []),
    models: () => Promise.reject(Object.assign(new Error("network error"), {
      body: '{"error":{"message":"catalog unavailable"}}',
    })),
  });
  const screen = host();

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.deepEqual(texts(screen.blocks), ["Fake: network error · catalog unavailable"]);
});

test("an unreachable Ollama API catalogue gives network retry guidance", async () => {
  const live = session({
    ...provider("ollama", []),
    models: () => Promise.reject(
      new Error("network error calling https://ollama.com/v1/models: fetch failed"),
    ),
  });
  const screen = host();

  await modelsCommand(live, screen, { save: false }, [live.provider]);

  assert.deepEqual(texts(screen.blocks), [
    "Ollama API: network request failed · check the connection and retry",
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
