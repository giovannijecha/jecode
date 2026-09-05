import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { ToolContext } from "../src/tools/index.ts";
import { atomicWrite } from "../src/atomic.ts";
import { readFile } from "../src/tools/file-read.ts";
import { editFile, runEditFile, runWriteFile, writeFile } from "../src/tools/file-write.ts";
import type { FileMutationDependencies } from "../src/tools/file-write.ts";
import { MAX_EDITABLE_BYTES, MAX_EDITABLE_LINES } from "../src/tools/text-boundary.ts";

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

test("writes and truncates a file to truly empty content", async () => {
  await writeFile.run({ path: "empty-write.txt", content: "before" }, ctx);
  const result = await writeFile.run({ path: "empty-write.txt", content: "" }, ctx);
  assert.equal(await fs.readFile(path.join(ctx.root, "empty-write.txt"), "utf8"), "");
  assert.equal(result.summary, "empty");
});

test("refuses to replace an existing file above the whole-file mutation limit", async () => {
  const file = path.join(ctx.root, "too-large-to-replace.txt");
  await fs.writeFile(file, "x".repeat(MAX_EDITABLE_BYTES + 1), "utf8");

  await assert.rejects(
    writeFile.run({ path: "too-large-to-replace.txt", content: "small" }, ctx),
    /whole-file mutation limit/,
  );
  assert.equal((await fs.stat(file)).size, MAX_EDITABLE_BYTES + 1);
});

test("refuses whole-file content with too many lines", async () => {
  const file = path.join(ctx.root, "too-many-lines.txt");
  const content = "line\n".repeat(MAX_EDITABLE_LINES);

  await assert.rejects(
    writeFile.run({ path: "too-many-lines.txt", content }, ctx),
    /20000 lines/,
  );
  await assert.rejects(fs.stat(file));
});

test("refuses to mutate a non-regular file", async () => {
  await fs.mkdir(path.join(ctx.root, "not-a-file"), { recursive: true });

  await assert.rejects(
    writeFile.run({ path: "not-a-file", content: "replacement" }, ctx),
    /regular file/,
  );
});

test("edits an exact string", async () => {
  await writeFile.run({ path: "edit.txt", content: "keep this, change that" }, ctx);
  await editFile.run({ path: "edit.txt", old_text: "change that", new_text: "changed it" }, ctx);
  assert.equal((await readFile.run({ path: "edit.txt" }, ctx)).output, "keep this, changed it");
});

test("edits replacement text containing JavaScript replacement tokens literally", async () => {
  await writeFile.run({ path: "replacement.txt", content: "echo hello\necho hello\n" }, ctx);
  const replacement = "echo pid=$$ && echo '$&' && echo '$`' && echo \"$'\"";
  await editFile.run({
    path: "replacement.txt",
    old_text: "echo hello",
    new_text: replacement,
    replace_all: true,
  }, ctx);
  assert.equal(
    await fs.readFile(path.join(ctx.root, "replacement.txt"), "utf8"),
    `${replacement}\n${replacement}\n`,
  );
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

test("rejects an expansive replace_all before constructing the result", async () => {
  const file = path.join(ctx.root, "expansive-edit.txt");
  const before = "a".repeat(1_000);
  await fs.writeFile(file, before, "utf8");

  await assert.rejects(
    editFile.run(
      {
        path: "expansive-edit.txt",
        old_text: "a",
        new_text: "b".repeat(1_001),
        replace_all: true,
      },
      ctx,
    ),
    /1000000 characters/,
  );
  assert.equal(await fs.readFile(file, "utf8"), before);
});

test("refuses an edit whose target is absent", async () => {
  await writeFile.run({ path: "miss.txt", content: "abc" }, ctx);
  await assert.rejects(
    editFile.run({ path: "miss.txt", old_text: "zzz", new_text: "!" }, ctx),
    /was not found/,
  );
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
  assert.equal(look?.beforeExists, false);
  // Previewing must never be the thing that creates the file.
  await assert.rejects(fs.stat(path.join(ctx.root, "fresh.txt")));
});

test("a write distinguishes a newly created empty file from the approved missing target", async () => {
  const args = { path: "created-after-preview.txt", content: "replacement" };
  const preview = await writeFile.preview?.(args, ctx);
  await fs.writeFile(path.join(ctx.root, args.path), "", "utf8");

  await assert.rejects(writeFile.run(args, { ...ctx, preview }), /changed after the preview/);
  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "");
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

test("a write preserves a concurrent update made immediately before rename", async () => {
  const args = { path: "commit-raced-write.txt", content: "approved replacement" };
  await writeFile.run({ path: args.path, content: "before" }, ctx);
  const preview = await writeFile.preview?.(args, ctx);

  await assert.rejects(
    runWriteFile(args, { ...ctx, preview }, replaceBeforeRename("concurrent update")),
    /changed after the preview/,
  );
  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "concurrent update");
});

test("a write preserves an empty file created immediately before rename", async () => {
  const args = { path: "commit-created-write.txt", content: "approved replacement" };
  const preview = await writeFile.preview?.(args, ctx);

  await assert.rejects(
    runWriteFile(args, { ...ctx, preview }, replaceBeforeRename("")),
    /changed after the preview/,
  );
  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "");
});

test("an edit preserves a concurrent update made immediately before rename", async () => {
  const args = { path: "commit-raced-edit.txt", old_text: "before", new_text: "after" };
  await writeFile.run({ path: args.path, content: "before" }, ctx);
  const preview = await editFile.preview?.(args, ctx);

  await assert.rejects(
    runEditFile(args, { ...ctx, preview }, replaceBeforeRename("concurrent update")),
    /changed after the preview/,
  );
  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "concurrent update");
});

test("an interrupted write cannot commit after reaching the rename boundary", async () => {
  const args = { path: "interrupted-write.txt", content: "after write" };
  await writeFile.run({ path: args.path, content: "before write" }, ctx);
  const preview = await writeFile.preview?.(args, ctx);
  const control = new AbortController();
  const gate = holdBeforeRename();
  const operation = runWriteFile(
    args,
    { ...ctx, preview, signal: control.signal },
    gate.dependencies,
  );

  await gate.reached;
  control.abort(new Error("stop write"));
  const rejected = assert.rejects(operation, /stop write/);
  gate.release();
  await rejected;

  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "before write");
  assert.equal((await fs.readdir(ctx.root)).some((name) => name.startsWith(`.${args.path}.`)), false);
});

test("an interrupted edit cannot commit after reaching the rename boundary", async () => {
  const args = { path: "interrupted-edit.txt", old_text: "before", new_text: "after edit" };
  await writeFile.run({ path: args.path, content: "before" }, ctx);
  const preview = await editFile.preview?.(args, ctx);
  const control = new AbortController();
  const gate = holdBeforeRename();
  const operation = runEditFile(
    args,
    { ...ctx, preview, signal: control.signal },
    gate.dependencies,
  );

  await gate.reached;
  control.abort(new Error("stop edit"));
  const rejected = assert.rejects(operation, /stop edit/);
  gate.release();
  await rejected;

  assert.equal(await fs.readFile(path.join(ctx.root, args.path), "utf8"), "before");
  assert.equal((await fs.readdir(ctx.root)).some((name) => name.startsWith(`.${args.path}.`)), false);
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

function replaceBeforeRename(content: string): FileMutationDependencies {
  return {
    atomicWrite: (file, replacement, options = {}) => atomicWrite(file, replacement, {
      ...options,
      async validate(phase) {
        if (phase === "before-rename") await fs.writeFile(file, content, "utf8");
        await options.validate?.(phase);
      },
    }),
  };
}

function holdBeforeRename(): {
  dependencies: FileMutationDependencies;
  reached: Promise<void>;
  release(): void;
} {
  let markReached: (() => void) | undefined;
  let resume: (() => void) | undefined;
  const reached = new Promise<void>((resolve) => { markReached = resolve; });
  const released = new Promise<void>((resolve) => { resume = resolve; });
  return {
    reached,
    release: () => resume?.(),
    dependencies: {
      atomicWrite: (file, replacement, options = {}) => atomicWrite(file, replacement, {
        ...options,
        async validate(phase) {
          if (phase === "before-rename") {
            markReached?.();
            await released;
          }
          await options.validate?.(phase);
        },
      }),
    },
  };
}
