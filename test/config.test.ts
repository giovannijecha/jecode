import { test } from "node:test";
import assert from "node:assert/strict";
import { loadConfig } from "../src/config.ts";

const CONFIG_ENV = [
  "JECODE_PROVIDER",
  "JECODE_MODEL",
  "OLLAMA_HOST",
  "JECODE_EFFORT",
  "JECODE_MAX_TOKENS",
  "JECODE_MAX_STEPS",
  "JECODE_COMPACTION_PERCENT",
  "JECODE_REDUCED_MOTION",
  "JECODE_AUTO_APPROVE",
  "JECODE_EPHEMERAL",
] as const;

const config = (argv: string[], saved = {}) => {
  const before = new Map(CONFIG_ENV.map((name) => [name, process.env[name]]));
  for (const name of CONFIG_ENV) delete process.env[name];
  try {
    return loadConfig(argv, saved);
  } finally {
    for (const [name, value] of before) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }
};

test("falls back to defaults", () => {
  const defaults = config([]);
  assert.equal(defaults.providerId, "anthropic");
  assert.equal(defaults.effort, "high");
  assert.equal(defaults.maxTokens, 64000);
  assert.equal(defaults.compactionPercent, 85);
  assert.equal(defaults.autoApprove, false);
  assert.equal(defaults.reducedMotion, false);
  assert.equal(defaults.ephemeral, false);
});

test("reads --key value and --key=value alike", () => {
  assert.equal(config(["--provider", "openai"]).providerId, "openai");
  assert.equal(config(["--provider=openai"]).providerId, "openai");
  assert.equal(config(["--provider", "openai-codex"]).providerId, "openai-codex");
});

test("treats a bare flag as true", () => {
  assert.equal(config(["--auto-approve"]).autoApprove, true);
  assert.equal(config(["--auto-approve", "--provider", "openai"]).autoApprove, true);
  assert.equal(config(["--reduced-motion"]).reducedMotion, true);
  assert.equal(config(["--reduced-motion=false"]).reducedMotion, false);
  assert.equal(config(["--reduced-motion", "false"]).reducedMotion, false);
  assert.equal(config(["--ephemeral"]).ephemeral, true);
});

test("rejects an unknown effort", () => {
  assert.throws(() => config(["--effort", "turbo"]), /unknown effort/);
});

test("rejects a non-positive integer budget", () => {
  assert.throws(() => config(["--max-tokens", "0"]), /positive integer/);
  assert.throws(() => config(["--max-steps", "n"]), /positive integer/);
});

test("accepts only a safe context compaction percentage", () => {
  assert.equal(config(["--compaction-percent", "90"]).compactionPercent, 90);
  assert.throws(() => config(["--compaction-percent", "49"]), /50 to 95/);
  assert.throws(() => config(["--compaction-percent", "96"]), /50 to 95/);
});

test("a flag nobody declared is refused, not swallowed", () => {
  assert.throws(() => config(["--sandbox"]), /unknown flag --sandbox/);
  assert.throws(() => config(["--modell", "gpt-5"]), /unknown flag --modell/);
  assert.throws(() => config(["--theme", "light"]), /unknown flag --theme/);
  assert.throws(() => config(["--palette", "violet"]), /unknown flag --palette/);
});

test("the flags that do exist still parse", () => {
  const parsed = config([
    "--provider",
    "openai",
    "--model=gpt-5",
    "--auto-approve",
  ]);
  assert.equal(parsed.providerId, "openai");
  assert.equal(parsed.model, "gpt-5");
  assert.equal(parsed.autoApprove, true);
});

test("rejects missing flag values and stray positional arguments", () => {
  assert.throws(() => config(["--root"]), /--root requires a value/);
  assert.throws(() => config(["--model"]), /--model requires a value/);
  assert.throws(() => config(["--provider="]), /--provider requires a value/);
  assert.throws(() => config(["--auto-approve=perhaps"]), /must be true or false/);
  assert.throws(() => config(["positional"]), /unexpected argument "positional"/);
  assert.throws(
    () => config(["--auto-approve", "typo"]),
    /unexpected argument "typo"/,
  );
});

test("saved defaults include a model for each provider", () => {
  const saved = {
    provider: "ollama",
    models: { anthropic: "claude-saved", ollama: "qwen-saved" },
    ollamaHost: "https://models.example.test/team/",
    effort: "medium",
    maxTokens: 8192,
    maxSteps: 12,
    compactionPercent: 92,
    reducedMotion: true,
  };
  const loaded = config([], saved);
  assert.equal(loaded.providerId, "ollama");
  assert.equal(loaded.model, "qwen-saved");
  assert.equal(loaded.ollamaHost, "https://models.example.test/team");
  assert.equal(loaded.effort, "medium");
  assert.equal(loaded.maxTokens, 8192);
  assert.equal(loaded.maxSteps, 12);
  assert.equal(loaded.compactionPercent, 92);
  assert.equal(loaded.reducedMotion, true);
});

test("Ollama host precedence is flag, environment, then saved settings", () => {
  const before = process.env["OLLAMA_HOST"];
  process.env["OLLAMA_HOST"] = "https://environment.example.test";
  try {
    const saved = { ollamaHost: "https://saved.example.test" };
    assert.equal(loadConfig([], saved).ollamaHost, "https://environment.example.test");
    assert.equal(
      loadConfig(["--ollama-host", "https://flag.example.test/path/"], saved).ollamaHost,
      "https://flag.example.test/path",
    );
  } finally {
    if (before === undefined) delete process.env["OLLAMA_HOST"];
    else process.env["OLLAMA_HOST"] = before;
  }
});

test("unsafe remote Ollama hosts are rejected at launch", () => {
  assert.throws(
    () => config(["--ollama-host", "http://models.example.test"]),
    /must use HTTPS/,
  );
});

test("flags and environment override saved defaults", () => {
  const before = process.env["JECODE_PROVIDER"];
  process.env["JECODE_PROVIDER"] = "openai";
  try {
    const loaded = loadConfig(["--provider", "ollama", "--model", "flag-model"], {
      provider: "anthropic",
      models: { anthropic: "saved-model" },
    });
    assert.equal(loaded.providerId, "ollama");
    assert.equal(loaded.model, "flag-model");
  } finally {
    if (before === undefined) delete process.env["JECODE_PROVIDER"];
    else process.env["JECODE_PROVIDER"] = before;
  }
});
