import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import * as path from "node:path";
import {
  credentialSource,
  credentialValues,
  forgetSaved,
  forgetSession,
  hasSaved,
  hold,
  keep,
  keyFor,
  reload,
  storeLabel,
  storePath,
} from "../src/credentials.ts";
import { USER_STORE_LIMITS } from "../src/user-store.ts";

const VAR = "JECODE_TEST_KEY";

/** Run with the store pointed at a directory of its own, then put it back. */
async function inStore(body: () => Promise<void> | void): Promise<void> {
  const dir = await mkdtemp(path.join(tmpdir(), "jecode-"));
  const before = {
    jecodeHome: process.env["JECODE_HOME"],
    appData: process.env["APPDATA"],
    xdg: process.env["XDG_CONFIG_HOME"],
  };

  process.env["JECODE_HOME"] = dir;
  process.env["APPDATA"] = dir;
  process.env["XDG_CONFIG_HOME"] = dir;
  reload();

  try {
    await body();
  } finally {
    restore("JECODE_HOME", before.jecodeHome);
    restore("APPDATA", before.appData);
    restore("XDG_CONFIG_HOME", before.xdg);
    reload();
    await rm(dir, { recursive: true, force: true });
  }
}

function restore(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}

test("the environment beats a key held for the session", async () => {
  await inStore(() => {
    hold(VAR, "from-session");
    process.env[VAR] = "from-env";
    try {
      assert.equal(keyFor(VAR), "from-env");
    } finally {
      delete process.env[VAR];
    }
    assert.equal(keyFor(VAR), "from-session");
  });
});

test("the environment beats a key on disk", async () => {
  await inStore(async () => {
    await keep(VAR, "from-disk");
    reload();

    process.env[VAR] = "from-env";
    try {
      assert.equal(keyFor(VAR), "from-env");
    } finally {
      delete process.env[VAR];
    }
    assert.equal(keyFor(VAR), "from-disk");
  });
});

test("an exported empty string is not a key", async () => {
  await inStore(() => {
    hold(VAR, "real");
    process.env[VAR] = "";
    try {
      assert.equal(keyFor(VAR), "real");
    } finally {
      delete process.env[VAR];
    }
  });
});

test("a held key is never written down", async () => {
  await inStore(async () => {
    hold(VAR, "secret");
    await assert.rejects(() => readFile(storePath(), "utf8"));
  });
});

test("a kept key survives a reload, and keeps the others", async () => {
  await inStore(async () => {
    await keep(VAR, "one");
    await keep(`${VAR}_2`, "two");
    reload();

    assert.equal(keyFor(VAR), "one");
    assert.equal(keyFor(`${VAR}_2`), "two");
  });
});

test("saving a replacement makes it active over an older session key", async () => {
  await inStore(async () => {
    hold(VAR, "old-session-value");
    await keep(VAR, "new-saved-value");

    assert.equal(credentialSource(VAR), "saved");
    assert.equal(keyFor(VAR), "new-saved-value");
  });
});

test("concurrent credential updates preserve every saved key", async () => {
  await inStore(async () => {
    await Promise.all([
      keep(VAR, "one"),
      keep(`${VAR}_2`, "two"),
    ]);
    reload();

    assert.equal(keyFor(VAR), "one");
    assert.equal(keyFor(`${VAR}_2`), "two");
  });
});

test("nonsense in the store is not a key", async () => {
  await inStore(async () => {
    await keep(VAR, "fine");
    reload();
    // Whatever else ends up in that file, only strings are credentials.
    const parsed = JSON.parse(await readFile(storePath(), "utf8")) as Record<string, unknown>;
    assert.deepEqual(Object.values(parsed), ["fine"]);
  });
});

test("the store is named the short way for a menu row", () => {
  const before = process.env["JECODE_HOME"];
  delete process.env["JECODE_HOME"];
  try {
    assert.ok(storeLabel().length <= storePath().length);
    assert.equal(path.dirname(storePath()), path.join(homedir(), ".jecode"));
    assert.match(storeLabel(), /\.jecode/);
  } finally {
    if (before !== undefined) process.env["JECODE_HOME"] = before;
  }
});

test("an old config-directory store remains a read-only fallback", async () => {
  await inStore(async () => {
    const legacy = path.join(process.env["APPDATA"] as string, "jecode", "credentials.json");
    await mkdir(path.dirname(legacy), { recursive: true });
    await writeFile(legacy, JSON.stringify({ [VAR]: "legacy-secret" }), "utf8");
    reload();

    assert.equal(keyFor(VAR), "legacy-secret");
    await assert.rejects(() => readFile(storePath(), "utf8"));
  });
});

test("an oversized canonical store cannot resurrect a legacy credential", async () => {
  await inStore(async () => {
    const legacy = path.join(process.env["APPDATA"] as string, "jecode", "credentials.json");
    await mkdir(path.dirname(legacy), { recursive: true });
    await writeFile(legacy, JSON.stringify({ [VAR]: "legacy-secret" }), "utf8");
    await writeFile(storePath(), "x".repeat(USER_STORE_LIMITS.credentialsBytes + 1), "utf8");
    reload();

    assert.equal(keyFor(VAR), undefined);
    assert.deepEqual(credentialValues(), []);
  });
});

test("credential stores bound entry count, names, and values", async () => {
  await inStore(async () => {
    const tooMany = Object.fromEntries(Array.from(
      { length: USER_STORE_LIMITS.credentialEntries + 1 },
      (_, index) => [`KEY_${index}`, `secret-${index}`],
    ));
    await writeFile(storePath(), JSON.stringify(tooMany), "utf8");
    reload();
    assert.equal(keyFor("KEY_0"), undefined);

    await writeFile(storePath(), JSON.stringify({
      [VAR]: "x".repeat(USER_STORE_LIMITS.credentialValue + 1),
      "invalid-name": "secret",
    }), "utf8");
    reload();
    assert.deepEqual(credentialValues(), []);
  });
});

test("new session credentials are refused before exceeding their bounds", async () => {
  await inStore(() => {
    for (let index = 0; index < USER_STORE_LIMITS.credentialEntries; index++) {
      hold(`KEY_${index}`, `secret-${index}`);
    }
    assert.throws(() => hold("ONE_TOO_MANY", "secret"), /too many session credentials/);
    assert.throws(
      () => hold("TOO_LONG", "x".repeat(USER_STORE_LIMITS.credentialValue + 1)),
      /invalid credential/,
    );
  });
});

test("a write beyond the credential file cap leaves the previous store intact", async () => {
  await inStore(async () => {
    const existing = Object.fromEntries(Array.from(
      { length: 15 },
      (_, index) => [
        `KEY_${index}`,
        String.fromCharCode(65 + index).repeat(USER_STORE_LIMITS.credentialValue),
      ],
    ));
    await writeFile(storePath(), JSON.stringify(existing), "utf8");
    reload();
    const before = await readFile(storePath(), "utf8");

    await assert.rejects(
      keep("KEY_15", "x".repeat(USER_STORE_LIMITS.credentialValue)),
      /user store exceeds/,
    );
    assert.equal(await readFile(storePath(), "utf8"), before);
  });
});

test("reports the winning credential source without exposing its value", async () => {
  await inStore(async () => {
    assert.equal(credentialSource(VAR), undefined);
    await keep(VAR, "saved-secret");
    reload();
    assert.equal(credentialSource(VAR), "saved");
    assert.equal(hasSaved(VAR), true);

    hold(VAR, "session-secret");
    assert.equal(credentialSource(VAR), "session");
    process.env[VAR] = "environment-secret";
    try {
      assert.equal(credentialSource(VAR), "environment");
    } finally {
      delete process.env[VAR];
    }
  });
});

test("forgets only the saved copy", async () => {
  await inStore(async () => {
    await keep(VAR, "saved-secret");
    hold(VAR, "session-secret");
    assert.equal(await forgetSaved(VAR), true);
    assert.equal(await forgetSaved(VAR), false);
    assert.equal(hasSaved(VAR), false);
    assert.equal(keyFor(VAR), "session-secret");

    const parsed = JSON.parse(await readFile(storePath(), "utf8")) as Record<string, string>;
    assert.equal(parsed[VAR], undefined);
  });
});

test("forgets only the session copy and reveals the saved fallback", async () => {
  await inStore(async () => {
    await keep(VAR, "saved-secret");
    hold(VAR, "session-secret");

    assert.equal(credentialSource(VAR), "session");
    assert.equal(forgetSession(VAR), true);
    assert.equal(forgetSession(VAR), false);
    assert.equal(credentialSource(VAR), "saved");
    assert.equal(keyFor(VAR), "saved-secret");
  });
});

test("the saved credential file is owner-only on POSIX", { skip: process.platform === "win32" }, async () => {
  await inStore(async () => {
    await keep(VAR, "secret");
    const fileMode = (await stat(storePath())).mode & 0o777;
    const directoryMode = (await stat(path.dirname(storePath()))).mode & 0o777;
    assert.equal(fileMode, 0o600);
    assert.equal(directoryMode, 0o700);
  });
});
