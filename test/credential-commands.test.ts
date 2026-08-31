import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import type { Host } from "../src/commands.ts";
import { credentialsCommand } from "../src/credential-commands.ts";
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
      maxSteps: 8,
      root: process.cwd(),
      autoApprove: false,
    },
    provider,
    model: provider.defaultModel,
    palette: STEEL,
    tools: [],
    system: "",
    history: [],
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

test("the credentials command can keep a missing key for this session", async () => {
  await inStore(async () => {
    const screen = host([1, 0, 0], "session-secret");

    await credentialsCommand(session(), screen);

    assert.equal(screen.fields[0]?.secret, true);
    assert.equal(credentialSource(KEY), "session");
    assert.equal(hasSaved(KEY), false);
    assert.match(screen.blocks.map((block) => "text" in block ? block.text : "").join("\n"), /this session/);
  });
});

test("the credentials command saves only after the explicit disk choice", async () => {
  await inStore(async () => {
    const screen = host([1, 0, 1], "saved-secret");

    await credentialsCommand(session(), screen);

    assert.equal(hasSaved(KEY), true);
    const stored = JSON.parse(await readFile(storePath(), "utf8")) as Record<string, string>;
    assert.equal(stored[KEY], "saved-secret");
    assert.ok(screen.blocks.every((block) => !("text" in block) || !block.text.includes("saved-secret")));
  });
});

test("an environment key can forget a shadowed saved copy without exposing either", async () => {
  await inStore(async () => {
    await keep(KEY, "saved-secret");
    reload();
    process.env[KEY] = "environment-secret";
    const screen = host([1, 1]);

    await credentialsCommand(session(), screen);

    assert.equal(credentialSource(KEY), "environment");
    assert.equal(hasSaved(KEY), false);
    const output = screen.blocks.map((block) => "text" in block ? block.text : "").join("\n");
    assert.match(output, /saved credential removed/);
    assert.doesNotMatch(output, /saved-secret|environment-secret/);
  });
});

test("the credentials command explains when no interactive screen is available", async () => {
  const blocks: Block[] = [];

  await credentialsCommand(session(), { emit: (block) => blocks.push(block) });

  assert.equal(blocks[0]?.kind, "notice");
  assert.match(blocks[0]?.kind === "notice" ? blocks[0].text : "", /needs the screen/);
});
