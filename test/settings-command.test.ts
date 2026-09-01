import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import { handleCommand } from "../src/commands.ts";
import type { Host } from "../src/commands.ts";
import { reloadAccounts } from "../src/accounts.ts";
import { loadConfig } from "../src/config.ts";
import { hold, reload as reloadCredentials } from "../src/credentials.ts";
import { modelsCommand } from "../src/model-command.ts";
import { OLLAMA_CLOUD_HOST, OLLAMA_LOCAL_HOST } from "../src/providers/ollama-endpoint.ts";
import { configureOllama, ollama, ollamaConnection } from "../src/providers/ollama.ts";
import { reloadSettings, settingsPath, updateSettings } from "../src/settings.ts";
import { settingsPicker } from "../src/settings-command.ts";
import type { Session } from "../src/session.ts";
import type { Field } from "../src/tui/field.ts";
import type { Picker } from "../src/tui/picker.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

test("settings uses nested dock pickers and persists a live change", async () => {
  await inSettingsHome(async () => {
    const pickers: Picker[] = [];
    const answers: (number | undefined)[] = [1, 1, undefined]; // effort, medium, escape
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
      "model",
      "effort",
      "max output tokens",
      "max tool steps",
      "context compaction",
      "reduced motion",
      "providers",
    ]);
    assert.equal(session.config.effort, "medium");
    assert.equal(JSON.parse(await readFile(settingsPath(), "utf8")).effort, "medium");
  });
});

test("context compaction percentage applies live and persists", async () => {
  await inSettingsHome(async () => {
    const answers: (number | undefined)[] = [4, undefined];
    const fields: Field[] = [];
    const host: Host = {
      emit: () => {},
      choose: () => Promise.resolve(answers.shift()),
      type: (field) => {
        fields.push(field);
        return Promise.resolve("90");
      },
    };
    const session = fakeSession();

    await handleCommand("/settings", session, host);

    assert.match(fields[0]?.note ?? "", /model context/);
    assert.equal(session.config.compactionPercent, 90);
    assert.equal(JSON.parse(await readFile(settingsPath(), "utf8")).compactionPercent, 90);
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

test("effort offers only the levels accepted by the selected model", async () => {
  await inSettingsHome(async () => {
    const pickers: Picker[] = [];
    const host: Host = {
      emit: () => {},
      choose: (picker) => {
        pickers.push(picker);
        return Promise.resolve(1);
      },
    };
    const session = fakeSession();
    session.provider.efforts = () => Promise.resolve(["low", "high"]);

    await handleCommand("/effort", session, host);

    assert.deepEqual(pickers[0]?.options.map((option) => option.label), ["low", "high"]);
    assert.equal(session.config.effort, "high");
  });
});

test("cancelling effort discovery propagates interruption without a provider notice", async () => {
  await inSettingsHome(async () => {
    const control = new AbortController();
    const notices: string[] = [];
    const host: Host = {
      emit: (block) => {
        if (block.kind === "notice") notices.push(block.text);
      },
      signal: control.signal,
      choose: () => Promise.resolve(undefined),
    };
    const session = fakeSession();
    session.provider.efforts = (_model, signal) =>
      new Promise<readonly string[]>((_resolve, reject) => {
        signal?.addEventListener("abort", () => reject(signal.reason), { once: true });
      });

    const pending = handleCommand("/effort", session, host);
    control.abort(new Error("interrupted"));

    await assert.rejects(pending, /interrupted/);
    assert.deepEqual(notices, []);
  });
});

test("OpenAI Codex settings omit a token limit the backend does not accept", () => {
  const picker = settingsPicker({
    provider: "openai-codex",
    model: "gpt-codex",
    effort: "high",
    maxSteps: 40,
    compactionPercent: 85,
    reducedMotion: false,
  });

  assert.equal(picker.options.some((option) => option.label === "max output tokens"), false);
  assert.equal(picker.options.some((option) => option.label === "providers"), true);
  assert.equal(picker.options[0]?.value, "ChatGPT · gpt-codex");
});

test("settings chooses provider and model together from the model menu", async () => {
  await inSettingsHome(async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (() => Promise.resolve(new Response(JSON.stringify({
      data: [{ id: "qwen3-coder" }],
    }), { status: 200 }))) as typeof fetch;
    const answers: (number | undefined)[] = [0, 0, undefined]; // model, first result, escape
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
      globalThis.fetch = originalFetch;
      configureOllama(undefined);
    }
  });
});

test("settings changes the Ollama connection live and persists it", async () => {
  await inSettingsHome(async () => {
    configureOllama(OLLAMA_CLOUD_HOST);
    const answers: (number | undefined)[] = [6, 3, 0, 1, undefined, undefined];
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

      assert.equal(pickers[0]?.options[6]?.label, "providers");
      assert.equal(pickers[2]?.options[0]?.label, "connection");
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
    const answers: (number | undefined)[] = [6, 3, 0, 0, 0, undefined, undefined];
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
    const answers: (number | undefined)[] = [6, 3, 0, 2, undefined, undefined];
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

test("a direct model choice becomes its provider and model on the next launch", async () => {
  await inSettingsHome(async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (() => Promise.resolve(new Response(JSON.stringify({
      data: [{ id: "qwen3-coder" }],
    }), { status: 200 }))) as typeof fetch;
    const host: Host = {
      emit: () => {},
      choose: () => Promise.resolve(0),
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
      await modelsCommand(session, host, {}, [ollama]);
      reloadSettings();
      const next = loadConfig([]);
      assert.equal(next.providerId, "ollama");
      assert.equal(next.model, "qwen3-coder");
    } finally {
      globalThis.fetch = originalFetch;
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
  const before = {
    home: process.env["JECODE_HOME"],
    anthropic: process.env["ANTHROPIC_API_KEY"],
    openai: process.env["OPENAI_API_KEY"],
  };
  process.env["JECODE_HOME"] = directory;
  delete process.env["ANTHROPIC_API_KEY"];
  delete process.env["OPENAI_API_KEY"];
  reloadSettings();
  reloadCredentials();
  reloadAccounts();
  try {
    await body();
  } finally {
    restore("JECODE_HOME", before.home);
    restore("ANTHROPIC_API_KEY", before.anthropic);
    restore("OPENAI_API_KEY", before.openai);
    reloadSettings();
    reloadCredentials();
    reloadAccounts();
    await rm(directory, { recursive: true, force: true });
  }
}

function restore(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
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
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider: {
      id: "fake",
      defaultModel: "model-a",
      auth: { kind: "api-key", keyVar: "FAKE_API_KEY" },
      blocked: () => undefined,
      models: () => Promise.resolve(["model-a"]),
      send: () => Promise.reject(new Error("not called")),
    },
    model: "model-a",
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}
