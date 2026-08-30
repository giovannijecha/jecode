import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { handleCommand } from "../src/commands.ts";
import type { Host } from "../src/commands.ts";
import { loadConfig } from "../src/config.ts";
import { hold, reload as reloadCredentials } from "../src/credentials.ts";
import { OLLAMA_CLOUD_HOST, OLLAMA_LOCAL_HOST } from "../src/providers/ollama-endpoint.ts";
import { configureOllama, ollama, ollamaConnection } from "../src/providers/ollama.ts";
import { reloadSettings, settingsPath, updateSettings } from "../src/settings.ts";
import type { Session } from "../src/session.ts";
import type { Field } from "../src/tui/field.ts";
import type { Picker } from "../src/tui/picker.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

test("settings uses nested dock pickers and persists a live change", async () => {
  await inSettingsHome(async () => {
    const pickers: Picker[] = [];
    const answers: (number | undefined)[] = [2, 1, 7]; // effort, medium, close
    const host: Host = {
      emit: () => {},
      choose: (picker) => {
        pickers.push(picker);
        return Promise.resolve(answers.shift());
      },
    };
    const session = fakeSession();

    await handleCommand("/settings", session, host);
    assert.deepEqual(pickers[0]?.options.map((option) => option.label), [
      "provider",
      "model",
      "effort",
      "max output tokens",
      "max tool steps",
      "reduced motion",
      "credentials",
      "close",
    ]);
    assert.equal(session.config.effort, "medium");
    assert.equal(JSON.parse(await readFile(settingsPath(), "utf8")).effort, "medium");
  });
});

test("effort is a direct picker that applies and persists the next-turn default", async () => {
  await inSettingsHome(async () => {
    const pickers: Picker[] = [];
    const notices: string[] = [];
    const host: Host = {
      emit: (block) => {
        if (block.kind === "notice") notices.push(block.text);
      },
      choose: (picker) => {
        pickers.push(picker);
        return Promise.resolve(4);
      },
    };
    const session = fakeSession();

    await handleCommand("/effort", session, host);

    assert.deepEqual(pickers[0]?.options.map((option) => option.label), [
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
    assert.equal(session.config.effort, "max");
    assert.equal(JSON.parse(await readFile(settingsPath(), "utf8")).effort, "max");
    assert.deepEqual(notices, ["effort · max"]);
  });
});

test("settings remembers a provider and its own model together", async () => {
  await inSettingsHome(async () => {
    await updateSettings({ models: { ollama: "qwen3-coder" } });
    const answers: (number | undefined)[] = [0, 2, 8]; // provider, ollama, close
    const host: Host = {
      emit: () => {},
      choose: () => Promise.resolve(answers.shift()),
    };
    const session = fakeSession();
    configureOllama(OLLAMA_LOCAL_HOST);

    try {
      await handleCommand("/settings", session, host);
      assert.equal(session.provider.id, "ollama");
      assert.equal(session.model, "qwen3-coder");
      const saved = JSON.parse(await readFile(settingsPath(), "utf8"));
      assert.equal(saved.provider, "ollama");
      assert.equal(saved.models.ollama, "qwen3-coder");
    } finally {
      configureOllama(undefined);
    }
  });
});

test("settings changes the Ollama connection live and persists it", async () => {
  await inSettingsHome(async () => {
    configureOllama(OLLAMA_CLOUD_HOST);
    const answers: (number | undefined)[] = [1, 1, 8]; // connection, local, close
    const pickers: Picker[] = [];
    const host: Host = {
      emit: () => {},
      choose: (picker) => {
        pickers.push(picker);
        return Promise.resolve(answers.shift());
      },
    };
    const session = fakeSession();
    session.provider = ollama;
    session.config.providerId = "ollama";

    try {
      await handleCommand("/settings", session, host);

      assert.equal(pickers[0]?.options[1]?.label, "ollama connection");
      assert.equal(session.config.ollamaHost, OLLAMA_LOCAL_HOST);
      assert.equal(ollamaConnection().baseUrl, OLLAMA_LOCAL_HOST);
      assert.equal(JSON.parse(await readFile(settingsPath(), "utf8")).ollamaHost, OLLAMA_LOCAL_HOST);
    } finally {
      configureOllama(undefined);
    }
  });
});

test("settings configures Ollama Cloud and collects a missing key in the dock", async () => {
  await inSettingsHome(async () => {
    configureOllama(OLLAMA_LOCAL_HOST);
    const answers: (number | undefined)[] = [1, 0, 0, 8]; // connection, cloud, session key, close
    const fields: Field[] = [];
    const host: Host = {
      emit: () => {},
      choose: () => Promise.resolve(answers.shift()),
      type: (field) => {
        fields.push(field);
        return Promise.resolve("fixture-cloud-key");
      },
    };
    const session = fakeSession();
    session.provider = ollama;
    session.config.providerId = "ollama";

    try {
      await handleCommand("/settings", session, host);

      assert.equal(fields[0]?.secret, true);
      assert.equal(session.config.ollamaHost, OLLAMA_CLOUD_HOST);
      assert.equal(ollamaConnection().baseUrl, OLLAMA_CLOUD_HOST);
      assert.equal(JSON.parse(await readFile(settingsPath(), "utf8")).ollamaHost, OLLAMA_CLOUD_HOST);
    } finally {
      configureOllama(undefined);
    }
  });
});

test("settings validates and normalizes a custom Ollama endpoint", async () => {
  await inSettingsHome(async () => {
    configureOllama(OLLAMA_LOCAL_HOST);
    hold("OLLAMA_API_KEY", "fixture-key");
    const answers: (number | undefined)[] = [1, 2, 8]; // connection, custom, close
    const fields: Field[] = [];
    const host: Host = {
      emit: () => {},
      choose: () => Promise.resolve(answers.shift()),
      type: (field) => {
        fields.push(field);
        return Promise.resolve("https://models.example.test/team/");
      },
    };
    const session = fakeSession();
    session.provider = ollama;
    session.config.providerId = "ollama";

    try {
      await handleCommand("/settings", session, host);

      assert.equal(fields[0]?.secret, false);
      assert.equal(session.config.ollamaHost, "https://models.example.test/team");
      assert.equal(ollamaConnection().kind, "custom");
      assert.equal(
        JSON.parse(await readFile(settingsPath(), "utf8")).ollamaHost,
        "https://models.example.test/team",
      );
    } finally {
      configureOllama(undefined);
    }
  });
});

test("a direct provider choice becomes the provider on the next launch", async () => {
  await inSettingsHome(async () => {
    await updateSettings({ models: { ollama: "qwen3-coder" } });
    const host: Host = {
      emit: () => {},
      choose: () => Promise.resolve(2),
      saveSettings: async (patch) => {
        await updateSettings(patch);
      },
    };
    const session = fakeSession();
    const beforeHost = process.env["OLLAMA_HOST"];
    const beforeProvider = process.env["JECODE_PROVIDER"];
    const beforeModel = process.env["JECODE_MODEL"];
    process.env["OLLAMA_HOST"] = "http://127.0.0.1:11434";
    configureOllama(OLLAMA_LOCAL_HOST);
    delete process.env["JECODE_PROVIDER"];
    delete process.env["JECODE_MODEL"];

    try {
      await handleCommand("/providers", session, host);
      reloadSettings();
      const next = loadConfig([]);
      assert.equal(next.providerId, "ollama");
      assert.equal(next.model, "qwen3-coder");
    } finally {
      if (beforeHost === undefined) delete process.env["OLLAMA_HOST"];
      else process.env["OLLAMA_HOST"] = beforeHost;
      configureOllama(undefined);
      if (beforeProvider === undefined) delete process.env["JECODE_PROVIDER"];
      else process.env["JECODE_PROVIDER"] = beforeProvider;
      if (beforeModel === undefined) delete process.env["JECODE_MODEL"];
      else process.env["JECODE_MODEL"] = beforeModel;
    }
  });
});

async function inSettingsHome(body: () => Promise<void>): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-settings-command-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadSettings();
  reloadCredentials();
  try {
    await body();
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadSettings();
    reloadCredentials();
    await rm(directory, { recursive: true, force: true });
  }
}

function fakeSession(): Session {
  return {
    config: {
      providerId: "fake",
      model: "model-a",
      reducedMotion: false,
      effort: "high",
      maxTokens: 4096,
      maxSteps: 8,
      root: process.cwd(),
      autoApprove: false,
    },
    provider: {
      id: "fake",
      defaultModel: "model-a",
      keyVar: "FAKE_API_KEY",
      blocked: () => undefined,
      models: () => Promise.resolve(["model-a"]),
      send: () => Promise.reject(new Error("not called")),
    },
    model: "model-a",
    palette: STEEL,
    tools: [],
    system: "",
    history: [],
    usage: emptyUsage(),
  };
}
