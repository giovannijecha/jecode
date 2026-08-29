import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { findFiles, searchText } from "../src/tools/search.ts";
import { builtinTools } from "../src/tools/index.ts";
import type { ToolContext } from "../src/tools/types.ts";

let ctx: ToolContext;

before(async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-search-"));
  ctx = { root };
  await fs.mkdir(path.join(root, "src", "nested"), { recursive: true });
  await fs.mkdir(path.join(root, "node_modules", "pkg"), { recursive: true });
  await fs.mkdir(path.join(root, ".git"), { recursive: true });
  await fs.writeFile(path.join(root, "src", "a.ts"), "const Needle = 1;\nsecond\n", "utf8");
  await fs.writeFile(path.join(root, "src", "nested", "b.ts"), "needle here\n", "utf8");
  await fs.writeFile(path.join(root, "readme.md"), "No needle in this sentence.\n", "utf8");
  await fs.writeFile(path.join(root, "node_modules", "pkg", "hidden.ts"), "needle\n", "utf8");
  await fs.writeFile(path.join(root, ".git", "secret.ts"), "needle\n", "utf8");
});

after(async () => {
  await fs.rm(ctx.root, { recursive: true, force: true });
});

test("find_files supports recursive globs and skips dependencies and VCS data", async () => {
  const result = await findFiles.run({ pattern: "**/*.ts" }, ctx);
  assert.deepEqual(result.output.split("\n"), ["src/a.ts", "src/nested/b.ts"]);
  assert.equal(result.summary, "2 files");
});

test("a basename glob also matches nested files", async () => {
  const result = await findFiles.run({ pattern: "*.md" }, ctx);
  assert.equal(result.output, "readme.md");
});

test("search_text reports relative path, line, and literal match", async () => {
  const result = await searchText.run({ query: "needle", pattern: "**/*.ts" }, ctx);
  assert.deepEqual(result.output.split("\n"), [
    "src/a.ts:1:const Needle = 1;",
    "src/nested/b.ts:1:needle here",
  ]);
  assert.equal(result.summary, "2 matches");
});

test("search_text can be case-sensitive and scoped to a directory", async () => {
  const result = await searchText.run(
    { query: "Needle", path: "src", case_sensitive: true },
    ctx,
  );
  assert.equal(result.output, "src/a.ts:1:const Needle = 1;");
});

test("search results stop at the requested bound", async () => {
  const result = await searchText.run({ query: "needle", max_results: 1 }, ctx);
  assert.equal(result.output.split("\n").length, 1);
  assert.match(result.summary ?? "", /result limit/);
});

test("search skips binary files", async () => {
  await fs.writeFile(path.join(ctx.root, "binary.dat"), Buffer.from([0, 110, 101, 101, 100, 108, 101]));
  const result = await searchText.run({ query: "needle", pattern: "*.dat" }, ctx);
  assert.equal(result.output, "[no matches]");
  assert.match(result.summary ?? "", /skipped 1/);
});

test("search never follows a junction outside the workspace", async (t) => {
  const outside = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-search-outside-"));
  try {
    await fs.writeFile(path.join(outside, "leak.ts"), "needle", "utf8");
    try {
      await fs.symlink(outside, path.join(ctx.root, "outside-link"), "junction");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating junctions is unavailable for this account");
        return;
      }
      throw error;
    }
    const result = await searchText.run({ query: "needle" }, ctx);
    assert.doesNotMatch(result.output, /leak\.ts/);
  } finally {
    await fs.rm(path.join(ctx.root, "outside-link"), { force: true }).catch(() => undefined);
    await fs.rm(outside, { recursive: true, force: true });
  }
});

test("an aborted search stops before walking", async () => {
  const control = new AbortController();
  control.abort(new Error("cancel search"));
  await assert.rejects(findFiles.run({ pattern: "*" }, { ...ctx, signal: control.signal }), /cancel search/);
});

test("the safe discovery tools are part of the built-in registry", () => {
  const names = builtinTools().map((tool) => tool.name);
  assert.ok(names.includes("find_files"));
  assert.ok(names.includes("search_text"));
});

test("search accepts an aliased workspace root", async (t) => {
  const area = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-search-alias-"));
  const workspace = path.join(area, "workspace");
  const alias = path.join(area, "alias");
  try {
    await fs.mkdir(workspace);
    await fs.writeFile(path.join(workspace, "found.ts"), "export const found = true;", "utf8");
    try {
      await fs.symlink(workspace, alias, process.platform === "win32" ? "junction" : "dir");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating a directory alias is unavailable for this account");
        return;
      }
      throw error;
    }

    const result = await findFiles.run({ pattern: "*.ts" }, { root: alias });
    assert.equal(result.output, "found.ts");
  } finally {
    await fs.rm(area, { recursive: true, force: true });
  }
});
