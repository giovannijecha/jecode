import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { editFile, listDir, readFile, writeFile } from "../src/tools/fs.ts";
import type { ToolContext } from "../src/tools/index.ts";

let ctx: ToolContext;

before(async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-test-"));
  ctx = { root };
});

after(async () => {
  await fs.rm(ctx.root, { recursive: true, force: true });
});

test("writes a file, creating parent directories", async () => {
  const { output: message } = await writeFile.run({ path: "src/deep/a.txt", content: "hello" }, ctx);
  assert.match(message, /src\/deep\/a\.txt/);
  assert.equal(await fs.readFile(path.join(ctx.root, "src/deep/a.txt"), "utf8"), "hello");
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

test("bounds a large read before returning it", async () => {
  await fs.writeFile(path.join(ctx.root, "large.txt"), "x".repeat(100_000), "utf8");
  const result = await readFile.run({ path: "large.txt" }, ctx);

  assert.match(result.output, /truncated at 60000 characters/);
  assert.equal(result.summary, "truncated at 60000 characters");
  assert.ok(result.output.length < 61_000);
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

test("edits an exact string", async () => {
  await writeFile.run({ path: "edit.txt", content: "keep this, change that" }, ctx);
  await editFile.run({ path: "edit.txt", old_text: "change that", new_text: "changed it" }, ctx);
  assert.equal((await readFile.run({ path: "edit.txt" }, ctx)).output, "keep this, changed it");
});

test("refuses an ambiguous edit unless replace_all is set", async () => {
  await writeFile.run({ path: "dup.txt", content: "x\nx\n" }, ctx);
  await assert.rejects(
    editFile.run({ path: "dup.txt", old_text: "x", new_text: "y" }, ctx),
    /appears 2 times/,
  );
  const { output: message } = await editFile.run(
    { path: "dup.txt", old_text: "x", new_text: "y", replace_all: true },
    ctx,
  );
  assert.match(message, /2 replacements/);
  assert.equal((await readFile.run({ path: "dup.txt" }, ctx)).output, "y\ny\n");
});

test("refuses an edit whose target is absent", async () => {
  await writeFile.run({ path: "miss.txt", content: "abc" }, ctx);
  await assert.rejects(
    editFile.run({ path: "miss.txt", old_text: "zzz", new_text: "!" }, ctx),
    /was not found/,
  );
});

test("lists a directory, marking subdirectories", async () => {
  await writeFile.run({ path: "listed/one.txt", content: "1" }, ctx);
  await writeFile.run({ path: "listed/sub/two.txt", content: "2" }, ctx);
  assert.equal((await listDir.run({ path: "listed" }, ctx)).output, "one.txt\nsub/");
});

test("refuses to touch anything outside the root", async () => {
  await assert.rejects(readFile.run({ path: "../escape.txt" }, ctx), /escapes the workspace root/);
  await assert.rejects(
    writeFile.run({ path: "../escape.txt", content: "no" }, ctx),
    /escapes the workspace root/,
  );
});

test("refuses reads and writes through a junction that leaves the root", async (t) => {
  const outside = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-outside-"));
  await fs.writeFile(path.join(outside, "secret.txt"), "secret", "utf8");
  try {
    try {
      await fs.symlink(outside, path.join(ctx.root, "outside-link"), "junction");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating junctions is unavailable for this account");
        return;
      }
      throw error;
    }

    await assert.rejects(
      readFile.run({ path: "outside-link/secret.txt" }, ctx),
      /escapes the workspace root/,
    );
    await assert.rejects(
      writeFile.run({ path: "outside-link/new.txt", content: "no" }, ctx),
      /escapes the workspace root/,
    );
    await assert.rejects(fs.stat(path.join(outside, "new.txt")));
  } finally {
    await fs.rm(path.join(ctx.root, "outside-link"), { force: true }).catch(() => undefined);
    await fs.rm(outside, { recursive: true, force: true });
  }
});

test("rejects a missing required argument with a message the model can act on", async () => {
  await assert.rejects(readFile.run({}, ctx), /"path" is required/);
});

test("a write over an existing file previews the replacement, not the file", async () => {
  await writeFile.run({ path: "conf.json", content: '{\n  "a": 1\n}\n' }, ctx);
  const look = await writeFile.preview?.({ path: "conf.json", content: '{\n  "a": 2\n}\n' }, ctx);

  assert.equal(look?.before, '{\n  "a": 1\n}\n');
  assert.equal(look?.after, '{\n  "a": 2\n}\n');
});

test("a write to a file that is not there yet previews as all new", async () => {
  const look = await writeFile.preview?.({ path: "fresh.txt", content: "hello" }, ctx);
  assert.equal(look?.before, "");
  assert.equal(look?.after, "hello");
  // Previewing must never be the thing that creates the file.
  await assert.rejects(fs.stat(path.join(ctx.root, "fresh.txt")));
});

test("an edit previews against the whole file, and leaves it alone", async () => {
  await writeFile.run({ path: "total.js", content: "let sum = 0;\nreturn sum;\n" }, ctx);
  const args = { path: "total.js", old_text: "let sum = 0;", new_text: "const sum = 0;" };
  const look = await editFile.preview?.(args, ctx);

  assert.equal(look?.before, "let sum = 0;\nreturn sum;\n");
  assert.equal(look?.after, "const sum = 0;\nreturn sum;\n");
  assert.equal(await fs.readFile(path.join(ctx.root, "total.js"), "utf8"), "let sum = 0;\nreturn sum;\n");
});

test("an edit that will not apply previews nothing rather than a fiction", async () => {
  await writeFile.run({ path: "one.js", content: "a\n" }, ctx);
  assert.equal(await editFile.preview?.({ path: "one.js", old_text: "zzz", new_text: "b" }, ctx), undefined);
});

test("a write refuses when the approved file changes before execution", async () => {
  const args = { path: "raced-write.txt", content: "approved replacement" };
  await writeFile.run({ path: args.path, content: "previewed" }, ctx);
  const preview = await writeFile.preview?.(args, ctx);
  await fs.writeFile(path.join(ctx.root, args.path), "changed elsewhere", "utf8");

  await assert.rejects(writeFile.run(args, { ...ctx, preview }), /changed after the preview/);
  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "changed elsewhere");
});

test("an edit refuses when the approved file changes before execution", async () => {
  const args = { path: "raced-edit.txt", old_text: "before", new_text: "after" };
  await writeFile.run({ path: args.path, content: "before" }, ctx);
  const preview = await editFile.preview?.(args, ctx);
  await fs.writeFile(path.join(ctx.root, args.path), "changed elsewhere", "utf8");

  await assert.rejects(editFile.run(args, { ...ctx, preview }), /changed after the preview/);
  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "changed elsewhere");
});

test("atomic replacement preserves an existing POSIX executable mode", { skip: process.platform === "win32" }, async () => {
  const file = path.join(ctx.root, "script.sh");
  await fs.writeFile(file, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  await editFile.run(
    { path: "script.sh", old_text: "exit 0", new_text: "exit 1" },
    ctx,
  );
  assert.equal((await fs.stat(file)).mode & 0o777, 0o755);
});
