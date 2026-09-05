import { test } from "node:test";
import assert from "node:assert/strict";
import * as path from "node:path";
import { loadConfig } from "../src/config.ts";
import type { SavedSettings } from "../src/settings.ts";

const RETIRED = {
  provider: "/models",
  model: "/models",
  effort: "/effort",
  "max-tokens": "/settings",
  "max-steps": "interactive turns have no request limit",
  "compaction-percent": "/settings",
  "auto-approve": "/permissions",
};
const CONFIG_ENV = [
  ...Object.keys(RETIRED).map((name) => "JECODE_" + name.toUpperCase().replaceAll("-", "_")),
  "OLLAMA_HOST", "JECODE_REDUCED_MOTION", "JECODE_EPHEMERAL",
];

function config(argv: string[], saved: SavedSettings = {}, environment: Record<string, string> = {}) {
  const before = new Map(CONFIG_ENV.map((name) => [name, process.env[name]]));
  for (const name of CONFIG_ENV) delete process.env[name];
  Object.assign(process.env, environment);
  try {
    return loadConfig(argv, saved);
  } finally {
    for (const [name, value] of before) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }
}

test("falls back to defaults without launch budgets or global approval", () => {
  assert.deepEqual(config([]), {
    providerId: "anthropic", model: "", effort: "high", maxTokens: 64000,
    compactionPercent: 85, root: process.cwd(), reducedMotion: false, ephemeral: false,
  });
});

test("workspace selection accepts both value forms", () => {
  assert.equal(config(["--root", "work"]).root, path.resolve("work"));
  assert.equal(config(["--root=work"]).root, path.resolve("work"));
});

test("process booleans accept bare and explicit values", () => {
  for (const flag of ["reduced-motion", "ephemeral"]) {
    const key = flag === "ephemeral" ? "ephemeral" : "reducedMotion";
    assert.equal(config(["--" + flag])[key], true);
    for (const value of ["true", "1", "false", "0"]) {
      const expected = value === "true" || value === "1";
      assert.equal(config(["--" + flag, value])[key], expected);
      assert.equal(config(["--" + flag + "=" + value])[key], expected);
    }
  }
});

test("retained process flags override environment and saved motion preferences", () => {
  const saved = { reducedMotion: true };
  assert.equal(config([], saved).reducedMotion, true);
  assert.equal(config([], saved, { JECODE_REDUCED_MOTION: "0" }).reducedMotion, false);
  assert.equal(config(["--reduced-motion"], saved, { JECODE_REDUCED_MOTION: "0" }).reducedMotion, true);
  assert.equal(config(["--reduced-motion=false"], saved, { JECODE_REDUCED_MOTION: "1" }).reducedMotion, false);
  assert.equal(config([], {}, { JECODE_EPHEMERAL: "1" }).ephemeral, true);
  assert.equal(config(["--ephemeral=false"], {}, { JECODE_EPHEMERAL: "1" }).ephemeral, false);
});

for (const [flag, guidance] of Object.entries(RETIRED)) {
  test("retired --" + flag + " gives replacement guidance without echoing values", () => {
    for (const args of [["--" + flag, "private-value"], ["--" + flag + "=private-value"], ["--" + flag]]) {
      assert.throws(() => config(args), (error: Error) => {
        assert.ok(error.message.includes("--" + flag + " is no longer supported"));
        assert.ok(error.message.includes(guidance));
        assert.ok(!error.message.includes("private-value"));
        return true;
      });
    }
  });

  test("retired " + flag + " environment cannot silently change saved defaults", () => {
    const variable = "JECODE_" + flag.toUpperCase().replaceAll("-", "_");
    assert.throws(() => config([], { provider: "openai", models: { openai: "saved-model" } }, {
      [variable]: "private-value",
    }), (error: Error) => {
      assert.ok(error.message.includes(variable + " is no longer supported; remove it"));
      assert.ok(error.message.includes(guidance));
      assert.ok(!error.message.includes("private-value"));
      return true;
    });
    assert.equal(config([], {}, { [variable]: "" }).providerId, "anthropic");
  });
}

test("unknown flags, missing values, and stray arguments are rejected", () => {
  for (const flag of ["sandbox", "modell", "theme", "palette", "__proto__"]) {
    assert.throws(() => config(["--" + flag]), /unknown flag/);
  }
  assert.throws(() => config(["--root"]), /--root requires a value/);
  assert.throws(() => config(["--root="]), /--root requires a value/);
  assert.throws(() => config(["--root", "--ephemeral"]), /--root requires a value/);
  assert.throws(() => config(["--ephemeral=perhaps"]), /must be true or false/);
  assert.throws(() => config(["positional"]), /unexpected argument/);
  assert.throws(() => config(["--ephemeral", "typo"]), /unexpected argument/);
});

test("saved defaults remain authoritative and preserve their source", () => {
  const saved = {
    provider: "ollama", models: { anthropic: "claude-saved", ollama: "qwen-saved" },
    ollamaHost: "https://ollama.com/", effort: "medium", maxTokens: 8192,
    compactionPercent: 92, reducedMotion: true,
  };
  const before = structuredClone(saved);
  assert.deepEqual(config([], saved), {
    providerId: "ollama", model: "qwen-saved", effort: "medium", maxTokens: 8192,
    compactionPercent: 92, reducedMotion: true, root: process.cwd(), ephemeral: false,
  });
  assert.deepEqual(saved, before);
});

test("invalid runtime settings retain meaningful validation", () => {
  assert.throws(() => config([], { effort: "turbo" }), /unknown effort/);
  for (const maxTokens of [0, Number.NaN, Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => config([], { maxTokens }), /positive safe integer/);
  }
  for (const compactionPercent of [49, 96, 85.5]) {
    assert.throws(() => config([], { compactionPercent }), /50 to 95/);
  }
});

test("retired official-cloud settings do not add runtime endpoints", () => {
  for (const host of ["https://ollama.com", " https://OLLAMA.COM:443/// "]) {
    assert.equal("ollamaHost" in config([], {}, { OLLAMA_HOST: host }), false);
    assert.equal("ollamaHost" in config([], { ollamaHost: host }), false);
  }
  assert.equal("ollamaHost" in config([], {}, { OLLAMA_HOST: "" }), false);
});

test("the retired endpoint flag fails without echoing its value", () => {
  for (const args of [
    ["--ollama-host", "https://ollama.com"],
    ["--ollama-host=https://user:private-value@models.example.test"],
    ["--ollama-host"],
  ]) {
    assert.throws(() => config(args), (error: Error) => {
      assert.match(error.message, /--ollama-host is no longer supported.*remove it/);
      assert.doesNotMatch(error.message, /private-value|models/);
      return true;
    });
  }
});

test("local, custom and malformed legacy endpoints cannot silently select cloud", () => {
  for (const host of [
    "http://127.0.0.1:11434", "http://localhost:11434", "http://[::1]:11434",
    "https://models.example.test", "https://user:private-value@ollama.com",
    "https://ollama.com/v1", "https://ollama.com?token=private-value",
    "https://ollama.com.example.test", "https://ollama.com:444", "not a URL",
  ]) {
    assert.throws(() => config([], { ollamaHost: "https://ollama.com" }, { OLLAMA_HOST: host }), (error: Error) => {
      assert.match(error.message, /OLLAMA_HOST.*remove it/);
      assert.doesNotMatch(error.message, /private-value|models|127/);
      return true;
    });
    assert.throws(() => config([], { ollamaHost: host }), (error: Error) => {
      assert.match(error.message, /settings.json.*remove ollamaHost/);
      assert.doesNotMatch(error.message, /private-value|models|127/);
      return true;
    });
  }
});
