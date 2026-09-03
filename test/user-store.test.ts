import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { readBoundedJsonSync } from "../src/user-store.ts";

test("a bounded JSON store reports its byte limit without exposing content", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-user-store-"));
  const file = path.join(directory, "store.json");
  try {
    await writeFile(file, "secret".repeat(20), "utf8");
    assert.throws(
      () => readBoundedJsonSync(file, 32),
      (error: unknown) => {
        assert.match((error as Error).message, /user store exceeds 32 bytes/);
        assert.doesNotMatch((error as Error).message, /secret/);
        return true;
      },
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
