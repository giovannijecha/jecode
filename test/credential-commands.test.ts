import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import type { Host } from "../src/commands.ts";
import { apiKeyCommand } from "../src/credential-commands.ts";
import {
  credentialSource,
  hasSaved,
  keep,
  reload,
  storePath,
} from "../src/credentials.ts";
import { PROVIDERS } from "../src/providers/index.ts";
import type { Session } from "../src/session.ts";
import type { Block } from "../src/tui/blocks.ts";
import type { Field } from "../src/tui/field.ts";
import type { Picker } from "../src/tui/picker.ts";
import { STEEL } from "../src/ui/theme.ts";
import { emptyUsage } from "../src/usage.ts";

const KEY = "OPENAI_API_KEY";

type Screen = Host & {
  blocks: Block[];
  fields: Field[];
  pickers: Picker[];
};

function session(): Session {
  const provider = PROVIDERS.find((candidate) => candidate.id === "anthropic");
  if (provider === undefined) throw new Error("anthropic provider fixture is missing");
  return {
    config: {
      providerId: provider.id,
      model: provider.defaultModel,
      reducedMotion: false,
      effort: "high",
      maxTokens: 4096,
      compactionPercent: 85,
      root: process.cwd(),
      autoApprove: false,
      ephemeral: false,
    },
    provider,
    model: provider.defaultModel,
    palette: STEEL,
    tools: [],
    system: "",
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };
}

function host(answers: (number | undefined)[], typed?: string): Screen {
  const screen: Screen = {
    blocks: [],
    fields: [],
    pickers: [],
    emit: (block) => screen.blocks.push(block),
    choose: (picker) => {
      screen.pickers.push(picker);
      return Promise.resolve(answers.shift());
    },
    type: (field) => {
      screen.fields.push(field);
      return Promise.resolve(typed);
    },
  };
  return screen;
}

async function inStore(body: () => Promise<void>): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-credential-command-"));
  const previous = {
    home: process.env["JECODE_HOME"],
    appData: process.env["APPDATA"],
    xdg: process.env["XDG_CONFIG_HOME"],
    key: process.env[KEY],
  };
  process.env["JECODE_HOME"] = directory;
  process.env["APPDATA"] = directory;
  process.env["XDG_CONFIG_HOME"] = directory;
  delete process.env[KEY];
  reload();
  try {
    await body();
  } finally {
    restore("JECODE_HOME", previous.home);
    restore("APPDATA", previous.appData);
    restore("XDG_CONFIG_HOME", previous.xdg);
    restore(KEY, previous.key);
    reload();
    await rm(directory, { recursive: true, force: true });
  }
}

function restore(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}

test("an API key can be kept for this session", async () => {
  await inStore(async () => {
    const screen = host([0, 0], "session-secret");

    await apiKeyCommand(KEY, "OpenAI API", session(), screen);

    assert.match(screen.pickers[0]?.title.map((part) => part.text).join("") ?? "", /^OpenAI API key  /);
    assert.equal(screen.fields[0]?.secret, true);
    assert.equal(credentialSource(KEY), "session");
    assert.equal(hasSaved(KEY), false);
    assert.match(screen.blocks.map((block) => "text" in block ? block.text : "").join("\n"), /this session/);
  });
});

test("adding an API key says explicitly whether that provider is active", async () => {
  await inStore(async () => {
    const inactive = host([0, 0], "session-secret");
    await apiKeyCommand(KEY, "OpenAI API", session(), inactive, "openai");
    assert.match(
      inactive.blocks.map((block) => "text" in block ? block.text : "").join("\n"),
      /choose OpenAI API in \/models to use it/,
    );

    const live = session();
    const selected = PROVIDERS.find((candidate) => candidate.id === "openai");
    if (selected === undefined) throw new Error("openai provider fixture is missing");
    live.provider = selected;
    live.config.providerId = selected.id;
    const active = host([0, 0], "replacement-secret");
    await apiKeyCommand(KEY, "OpenAI API", live, active, "openai");
    assert.match(
      active.blocks.map((block) => "text" in block ? block.text : "").join("\n"),
      /current provider route/,
    );
  });
});

test("API key actions rely on escape instead of a redundant close row", async () => {
  await inStore(async () => {
    const screen = host([undefined]);

    await apiKeyCommand(KEY, "OpenAI API", session(), screen);

    assert.deepEqual(
      screen.pickers[0]?.options.map((option) => option.label),
      ["add API key"],
    );
    assert.equal(screen.fields.length, 0);
  });
});

test("an API key is saved only after the explicit disk choice", async () => {
  await inStore(async () => {
    const screen = host([0, 1], "saved-secret");

    await apiKeyCommand(KEY, "OpenAI API", session(), screen);

    assert.equal(hasSaved(KEY), true);
    const stored = JSON.parse(await readFile(storePath(), "utf8")) as Record<string, string>;
    assert.equal(stored[KEY], "saved-secret");
    assert.ok(screen.blocks.every((block) => !("text" in block) || !block.text.includes("saved-secret")));
  });
});

test("discarding a typed credential is a silent cancellation", async () => {
  await inStore(async () => {
    const screen = host([0, 2], "discarded-secret");

    await apiKeyCommand(KEY, "OpenAI API", session(), screen);

    assert.equal(credentialSource(KEY), undefined);
    assert.deepEqual(screen.blocks, []);
  });
});

test("an environment key can forget a shadowed saved copy without exposing either", async () => {
  await inStore(async () => {
    await keep(KEY, "saved-secret");
    reload();
    process.env[KEY] = "environment-secret";
    const screen = host([0]);

    await apiKeyCommand(KEY, "OpenAI API", session(), screen);

    assert.match(screen.pickers[0]?.title.map((part) => part.text).join("") ?? "", /^OpenAI API key  /);
    assert.equal(credentialSource(KEY), "environment");
    assert.equal(hasSaved(KEY), false);
    const output = screen.blocks.map((block) => "text" in block ? block.text : "").join("\n");
    assert.match(output, /saved API key removed/);
    assert.doesNotMatch(output, /saved-secret|environment-secret/);
  });
});

test("an environment key names its API route without exposing the value", async () => {
  await inStore(async () => {
    process.env[KEY] = "environment-secret";
    const screen = host([]);

    await apiKeyCommand(KEY, "OpenAI API", session(), screen);

    assert.equal(credentialSource(KEY), "environment");
    assert.equal(screen.pickers.length, 0);
    assert.equal(screen.fields.length, 0);
    assert.deepEqual(screen.blocks, [{
      kind: "notice",
      text: "OpenAI API key comes from the environment · restart after changing it",
      tone: "info",
    }]);
  });
});

test("API key management explains when no interactive screen is available", async () => {
  const blocks: Block[] = [];

  await apiKeyCommand(KEY, "OpenAI API", session(), { emit: (block) => blocks.push(block) });

  assert.equal(blocks[0]?.kind, "notice");
  assert.match(blocks[0]?.kind === "notice" ? blocks[0].text : "", /needs the screen/);
});
