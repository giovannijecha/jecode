import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { ToolContext } from "../src/tools/index.ts";
import { execFile as execFileCallback } from "node:child_process";
import { promisify } from "node:util";
import { listDir, readFile } from "../src/tools/file-read.ts";
import { writeFile } from "../src/tools/file-write.ts";

let ctx: ToolContext;
const execFile = promisify(execFileCallback);

before(async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-test-"));
  ctx = { root };
});

after(async () => {
  await fs.rm(ctx.root, { recursive: true, force: true });
});

test("reads a file back, whole and by line range", async () => {
  await writeFile.run({ path: "lines.txt", content: "one\ntwo\nthree\nfour" }, ctx);
  assert.equal((await readFile.run({ path: "lines.txt" }, ctx)).output, "one\ntwo\nthree\nfour");
  assert.equal((await readFile.run({ path: "lines.txt", offset: 2, limit: 2 }, ctx)).output, "two\nthree");
});

test("reports an empty file rather than returning nothing", async () => {
  await writeFile.run({ path: "empty.txt", content: " " }, ctx);
  await fs.writeFile(path.join(ctx.root, "empty.txt"), "", "utf8");
  assert.equal((await readFile.run({ path: "empty.txt" }, ctx)).output, "[file is empty]");
});

test("refuses a FIFO without waiting for a writer", {
  skip: process.platform === "win32",
  timeout: 2_000,
}, async () => {
  const fifo = path.join(ctx.root, "read.pipe");
  await execFile("mkfifo", [fifo]);

  await assert.rejects(
    readFile.run({ path: "read.pipe" }, ctx),
    /regular file/,
  );
});

test("honors cancellation before reading file content", async () => {
  await fs.writeFile(path.join(ctx.root, "cancelled-read.txt"), "content", "utf8");
  const control = new AbortController();
  control.abort(new Error("stop reading"));

  await assert.rejects(
    readFile.run({ path: "cancelled-read.txt" }, { ...ctx, signal: control.signal }),
    /stop reading/,
  );
});

test("bounds a large read before returning it", async () => {
  await fs.writeFile(path.join(ctx.root, "large.txt"), "x".repeat(100_000), "utf8");
  const result = await readFile.run({ path: "large.txt" }, ctx);

  assert.match(result.output, /truncated at 60000 characters/);
  assert.equal(result.summary, "truncated at 60000 characters");
  assert.ok(result.output.length < 61_000);
});

test("a bounded read never returns half an emoji", async () => {
  const emoji = String.fromCodePoint(0x1f600);
  await fs.writeFile(
    path.join(ctx.root, "unicode-boundary.txt"),
    `${"x".repeat(59_999)}${emoji}tail`,
    "utf8",
  );

  const result = await readFile.run({ path: "unicode-boundary.txt" }, ctx);

  assert.equal(result.output.isWellFormed(), true);
  assert.equal(result.output.includes(emoji), false);
  assert.match(result.output, /truncated at 60000 characters/);
});

test("scans past an unselected large line without returning it", async () => {
  await fs.writeFile(
    path.join(ctx.root, "offset.txt"),
    `${"x".repeat(1_000_000)}\nwanted\nafter`,
    "utf8",
  );

  assert.equal(
    (await readFile.run({ path: "offset.txt", offset: 2, limit: 1 }, ctx)).output,
    "wanted",
  );
});

test("bounds work while seeking a line beyond a very large prefix", async () => {
  const file = path.join(ctx.root, "scan-capped.txt");
  const handle = await fs.open(file, "w");
  try {
    await handle.truncate(16 * 1024 * 1024 + 1);
  } finally {
    await handle.close();
  }

  const result = await readFile.run({ path: "scan-capped.txt", offset: 2, limit: 1 }, ctx);

  assert.match(result.output, /stopped after scanning 16777216 bytes/);
  assert.equal(result.summary, "scan capped at 16777216 bytes");
});

test("rejects integer arguments outside JavaScript's exact range", async () => {
  await fs.writeFile(path.join(ctx.root, "lines.txt"), "one\ntwo", "utf8");
  await assert.rejects(
    readFile.run({ path: "lines.txt", offset: Number.MAX_SAFE_INTEGER + 1 }, ctx),
    /"offset" must be a safe integer/,
  );
});

test("lists a directory, marking subdirectories", async () => {
  await writeFile.run({ path: "listed/one.txt", content: "1" }, ctx);
  await writeFile.run({ path: "listed/sub/two.txt", content: "2" }, ctx);
  assert.equal((await listDir.run({ path: "listed" }, ctx)).output, "one.txt\nsub/");
});

test("an omitted or empty list path means the workspace root", async () => {
  await writeFile.run({ path: "root-entry.txt", content: "root" }, ctx);
  const omitted = await listDir.run({}, ctx);
  const empty = await listDir.run({ path: "" }, ctx);

  assert.equal(empty.output, omitted.output);
  assert.match(empty.output, /(?:^|\n)root-entry\.txt(?:\n|$)/);
});

test("rejects a missing required argument with a message the model can act on", async () => {
  await assert.rejects(readFile.run({}, ctx), /"path" is required/);
});
