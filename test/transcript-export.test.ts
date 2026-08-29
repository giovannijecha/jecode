import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { saveTranscript } from "../src/transcript-export.ts";

test("automatic export writes a timestamped Markdown file directly in the application root", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-transcript-"));
  try {
    const saved = await saveTranscript(
      root,
      [{ kind: "user", text: "hello" }],
      new Date("2026-08-29T12:34:56.789Z"),
    );

    assert.equal(saved, "jecode-transcript-20260829T123456Z.md");
    const contents = await readFile(path.join(root, saved), "utf8");
    assert.match(contents, /## You\n\nhello/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("automatic export stays relative when the application root is an alias", async (t) => {
  const area = await mkdtemp(path.join(tmpdir(), "jecode-transcript-alias-"));
  const root = path.join(area, "root");
  const alias = path.join(area, "alias");
  try {
    await mkdir(root);
    try {
      await symlink(root, alias, process.platform === "win32" ? "junction" : "dir");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating a directory alias is unavailable for this account");
        return;
      }
      throw error;
    }

    const saved = await saveTranscript(
      alias,
      [{ kind: "user", text: "hello" }],
      new Date("2026-08-29T12:34:56.789Z"),
    );

    assert.equal(saved, "jecode-transcript-20260829T123456Z.md");
    assert.match(await readFile(path.join(root, saved), "utf8"), /## You\n\nhello/);
  } finally {
    await rm(area, { recursive: true, force: true });
  }
});
