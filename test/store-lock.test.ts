import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { withStoreLock } from "../src/store-lock.ts";

test("a stale timestamp never steals a lock from its live owner", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "accounts.json");
  const directory = `${file}.lock`;
  let entered = false;

  try {
    await mkdir(directory);
    await writeFile(path.join(directory, "owner"), `${process.pid}:fixture`, "utf8");
    const old = new Date(Date.now() - 120_000);
    await utimes(directory, old, old);

    await assert.rejects(
      withStoreLock(file, async () => {
        entered = true;
      }, AbortSignal.timeout(150)),
      /aborted|timeout/i,
    );
    assert.equal(entered, false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("an abandoned stale lock remains recoverable", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "settings.json");
  const directory = `${file}.lock`;
  let entered = false;

  try {
    const owner = spawn(process.execPath, ["-e", ""], {
      stdio: "ignore",
      windowsHide: true,
    });
    const pid = owner.pid as number;
    await once(owner, "exit");

    await mkdir(directory);
    await writeFile(path.join(directory, "owner"), `${pid}:fixture`, "utf8");
    const old = new Date(Date.now() - 120_000);
    await utimes(directory, old, old);

    await withStoreLock(file, async () => {
      entered = true;
    });
    assert.equal(entered, true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
