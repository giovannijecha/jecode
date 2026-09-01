import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
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
      ollamaHost: "https://models.example.test/team/",
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
      ollamaHost: "https://models.example.test/team",
      effort: "medium",
      reducedMotion: true,
      maxTokens: 8192,
      maxSteps: 12,
      compactionPercent: 90,
    });
    const saved = JSON.parse(await readFile(settingsPath(), "utf8"));
    assert.equal(saved.provider, "ollama");
    assert.equal(saved.ollamaHost, "https://models.example.test/team");
    assert.equal(saved.theme, undefined);
    assert.equal(saved.palette, undefined);
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
      ollamaHost: "http://models.example.test",
      effort: "turbo",
      maxTokens: -1,
      compactionPercent: 100,
    } as SavedSettings);
    reloadSettings();
    assert.deepEqual(readSettings(), {});
  });
});

test("the settings file is owner-only on POSIX", { skip: process.platform === "win32" }, async () => {
  await inSettingsHome(async () => {
    await updateSettings({ provider: "ollama" });
    assert.equal((await stat(settingsPath())).mode & 0o777, 0o600);
    assert.equal((await stat(path.dirname(settingsPath()))).mode & 0o777, 0o700);
  });
});
