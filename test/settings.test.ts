import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import * as path from "node:path";
import {
  readSettings,
  reloadSettings,
  settingsLabel,
  settingsPath,
  updateSettings,
} from "../src/settings.ts";
import type { SavedSettings } from "../src/settings.ts";
import { loadConfig } from "../src/config.ts";
import { USER_STORE_LIMITS } from "../src/user-store.ts";

async function inSettingsHome(body: () => Promise<void> | void): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-settings-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadSettings();
  try {
    await body();
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadSettings();
    await rm(directory, { recursive: true, force: true });
  }
}

test("the default settings store lives in the user's .jecode directory", () => {
  const before = process.env["JECODE_HOME"];
  delete process.env["JECODE_HOME"];
  try {
    assert.equal(path.dirname(settingsPath()), path.join(homedir(), ".jecode"));
    assert.match(settingsLabel(), /\.jecode/);
  } finally {
    if (before !== undefined) process.env["JECODE_HOME"] = before;
  }
});

test("saved defaults and provider-specific models survive a reload", async () => {
  await inSettingsHome(async () => {
    const legacyVisualSettings = {
      provider: "ollama",
      models: { anthropic: "claude-sonnet-5", ollama: "qwen3-coder" },
      effort: "medium",
      theme: "mono",
      palette: "violet",
      reducedMotion: true,
      maxTokens: 8192,
      maxSteps: 12,
      compactionPercent: 90,
    } as unknown as Partial<SavedSettings>;
    await updateSettings(legacyVisualSettings);
    reloadSettings();

    assert.deepEqual(readSettings(), {
      provider: "ollama",
      models: { anthropic: "claude-sonnet-5", ollama: "qwen3-coder" },
      effort: "medium",
      reducedMotion: true,
      maxTokens: 8192,
      compactionPercent: 90,
    });
    const saved = JSON.parse(await readFile(settingsPath(), "utf8"));
    assert.equal(saved.provider, "ollama");
    assert.equal(saved.ollamaHost, undefined);
    assert.equal(saved.theme, undefined);
    assert.equal(saved.palette, undefined);
    assert.equal(saved.maxSteps, undefined);
  });
});

test("concurrent settings updates preserve unrelated fields", async () => {
  await inSettingsHome(async () => {
    await Promise.all([
      updateSettings({ provider: "ollama" }),
      updateSettings({ effort: "high" }),
    ]);
    reloadSettings();

    assert.deepEqual(readSettings(), { provider: "ollama", effort: "high" });
  });
});

test("invalid saved values are discarded rather than breaking startup", async () => {
  await inSettingsHome(async () => {
    await updateSettings({
      provider: "unknown",
      models: { unknown: "model", ollama: "" },
      effort: "turbo",
      maxTokens: -1,
      compactionPercent: 100,
    } as SavedSettings);
    reloadSettings();
    assert.deepEqual(readSettings(), {});
  });
});

test("oversized and truncated settings stores fall back before parsing", async () => {
  await inSettingsHome(async () => {
    await writeFile(settingsPath(), "x".repeat(USER_STORE_LIMITS.settingsBytes + 1), "utf8");
    reloadSettings();
    assert.deepEqual(readSettings(), {});

    await writeFile(settingsPath(), '{"provider":"ollama"', "utf8");
    reloadSettings();
    assert.deepEqual(readSettings(), {});
  });
});

test("a settings mutation preserves a truncated existing store", async () => {
  await inSettingsHome(async () => {
    const before = '{"provider":"ollama"';
    await writeFile(settingsPath(), before, "utf8");
    reloadSettings();

    await assert.rejects(
      updateSettings({ effort: "high" }),
      /settings store is invalid, unsafe, or too large/,
    );
    assert.equal(await readFile(settingsPath(), "utf8"), before);
  });
});

test("a settings mutation preserves unsupported or invalid stored fields", async () => {
  await inSettingsHome(async () => {
    for (const invalid of [
      { futureSetting: true },
      { provider: "unknown" },
      { models: { ollama: "x".repeat(USER_STORE_LIMITS.model + 1) } },
    ]) {
      const before = JSON.stringify(invalid);
      await writeFile(settingsPath(), before, "utf8");
      reloadSettings();

      await assert.rejects(updateSettings({ effort: "high" }), /settings store has/);
      assert.equal(await readFile(settingsPath(), "utf8"), before);
    }
  });
});

test("settings discard oversized model strings but retain an invalid endpoint marker", async () => {
  await inSettingsHome(async () => {
    await writeFile(settingsPath(), JSON.stringify({
      models: { ollama: "x".repeat(USER_STORE_LIMITS.model + 1) },
      ollamaHost: `https://example.test/${"x".repeat(USER_STORE_LIMITS.endpoint)}`,
    }), "utf8");
    reloadSettings();

    assert.deepEqual(readSettings(), { ollamaHost: "unsupported legacy Ollama endpoint" });
  });
});

test("official-cloud legacy settings are retired on the next explicit write", async () => {
  await inSettingsHome(async () => {
    const before = JSON.stringify({ provider: "ollama", models: { ollama: "cloud-model" },
      ollamaHost: " https://OLLAMA.COM:443/// ", effort: "medium" });
    await writeFile(settingsPath(), before, "utf8");
    reloadSettings();
    assert.deepEqual(readSettings(), { provider: "ollama", models: { ollama: "cloud-model" },
      ollamaHost: "https://ollama.com", effort: "medium" });
    assert.equal(await readFile(settingsPath(), "utf8"), before, "reading does not migrate the store");
    await updateSettings({ effort: "high" });
    reloadSettings();
    const expected = { provider: "ollama", models: { ollama: "cloud-model" }, effort: "high" };
    assert.deepEqual(readSettings(), expected);
    assert.deepEqual(JSON.parse(await readFile(settingsPath(), "utf8")), expected);
  });
});

test("non-cloud legacy settings block startup and unrelated writes without losing the stored value", async () => {
  await inSettingsHome(async () => {
    const oldHost = process.env["OLLAMA_HOST"];
    delete process.env["OLLAMA_HOST"];
    try {
      for (const host of [
        "http://localhost:11434", "https://models.example.test/team", "http://models.example.test",
        "https://user:private-value@ollama.com", "not a URL", "", null, { endpoint: "private-value" },
        "x".repeat(USER_STORE_LIMITS.endpoint + 1),
      ]) {
        const before = JSON.stringify({ provider: "ollama", models: { ollama: "saved-model" }, ollamaHost: host, effort: "medium" });
        await writeFile(settingsPath(), before, "utf8");
        reloadSettings();
        const saved = readSettings();
        assert.equal(saved.provider, "ollama");
        assert.deepEqual(saved.models, { ollama: "saved-model" });
        assert.ok(saved.ollamaHost !== undefined, "normalization must retain the retired setting");
        if (typeof host === "string" && host !== "" && host.length <= USER_STORE_LIMITS.endpoint) {
          assert.equal(saved.ollamaHost, host);
        }
        assert.throws(() => loadConfig([], saved), (error: Error) => {
          assert.match(error.message, /settings\.json.*remove ollamaHost/);
          assert.doesNotMatch(error.message, /private-value|models\.example/);
          return true;
        });
        await assert.rejects(updateSettings({ effort: "high" }), (error: Error) => {
          assert.match(error.message, /retired Ollama endpoint.*remove ollamaHost/);
          assert.doesNotMatch(error.message, /private-value|models\.example/);
          return true;
        });
        assert.equal(await readFile(settingsPath(), "utf8"), before);
        assert.deepEqual(readSettings(), saved, "a rejected write also preserves the cached read");
      }
    } finally {
      if (oldHost === undefined) delete process.env["OLLAMA_HOST"];
      else process.env["OLLAMA_HOST"] = oldHost;
    }
  });
});

test("a retired non-cloud endpoint cannot be introduced through a settings patch", async () => {
  await inSettingsHome(async () => {
    await updateSettings({ effort: "medium" });
    const before = await readFile(settingsPath(), "utf8");
    await assert.rejects(updateSettings({ ollamaHost: "http://localhost:11434" }), /retired Ollama endpoint/);
    assert.equal(await readFile(settingsPath(), "utf8"), before);
    await updateSettings({ ollamaHost: "https://ollama.com" });
    assert.deepEqual(JSON.parse(await readFile(settingsPath(), "utf8")), { effort: "medium" });
  });
});

test("the settings file is owner-only on POSIX", { skip: process.platform === "win32" }, async () => {
  await inSettingsHome(async () => {
    await updateSettings({ provider: "ollama" });
    assert.equal((await stat(settingsPath())).mode & 0o777, 0o600);
    assert.equal((await stat(path.dirname(settingsPath()))).mode & 0o777, 0o700);
  });
});
