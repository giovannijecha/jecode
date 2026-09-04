import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { stableFileExpectation } from "../src/bounded-file.ts";
import { findFiles, portableSearch, searchText } from "../src/tools/search.ts";
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
  await fs.writeFile(path.join(root, "binary.dat"), Buffer.from([0, 110, 101, 101, 100, 108, 101]));
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

test("globstar can match zero directories and question matches one character", async () => {
  const rootFile = await findFiles.run({ pattern: "**/*.md" }, ctx);
  const oneCharacter = await findFiles.run({ pattern: "**/?.ts" }, ctx);
  assert.equal(rootFile.output, "readme.md");
  assert.deepEqual(oneCharacter.output.split("\n"), ["src/a.ts", "src/nested/b.ts"]);
});

test("rejects a glob too large to evaluate predictably", async () => {
  await assert.rejects(
    findFiles.run({ pattern: "a*".repeat(300) }, ctx),
    /must be at most 512 characters/,
  );
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

test("bounded search returns the canonical lexical prefix", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-search-order-"));
  const ordered = { root };
  try {
    await fs.mkdir(path.join(root, "a-dir"));
    await fs.mkdir(path.join(root, "z-dir"));
    await fs.writeFile(path.join(root, "a-dir", "hit.txt"), "ordered needle a\n", "utf8");
    await fs.writeFile(path.join(root, "b-hit.txt"), "ordered needle b\n", "utf8");
    await fs.writeFile(path.join(root, "z-dir", "hit.txt"), "ordered needle z\n", "utf8");

    const files = await findFiles.run({ pattern: "*.txt", max_results: 1 }, ordered);
    const matches = await searchText.run({ query: "ordered needle", max_results: 1 }, ordered);

    assert.equal(files.output, "a-dir/hit.txt");
    assert.equal(matches.output, "a-dir/hit.txt:1:ordered needle a");
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test("the portable scanner bounds concurrent reads and preserves file order", async () => {
  const files = Array.from({ length: 20 }, (_, index) => ({
    path: path.join(ctx.root, `ordered-${String(index).padStart(2, "0")}.txt`),
    bytes: 16,
  }));
  let active = 0;
  let peak = 0;
  const searched = await portableSearch(files, ctx, "needle", true, 5, async (file) => {
    active++;
    peak = Math.max(peak, active);
    try {
      const index = Number.parseInt(path.basename(file).slice(8, 10), 10);
      await new Promise((resolve) => setTimeout(resolve, 20 - index));
      return Buffer.from(`needle ${index}\n`, "utf8");
    } finally {
      active--;
    }
  });

  assert.equal(peak, 8);
  assert.deepEqual(
    searched.matches.map((match) => match.slice(match.lastIndexOf(":") + 1)),
    Array.from({ length: 5 }, (_, index) => `needle ${index}`),
  );
});

test("the portable scanner preserves the caller's cancellation reason", async () => {
  const control = new AbortController();
  const files = Array.from({ length: 8 }, (_, index) => ({
    path: path.join(ctx.root, `waiting-${index}.txt`),
    bytes: 16,
  }));
  const pending = portableSearch(
    files,
    { ...ctx, signal: control.signal },
    "needle",
    true,
    100,
    async (_file, signal) => await new Promise<Buffer>((_resolve, reject) => {
      signal?.addEventListener("abort", () => reject(signal.reason), { once: true });
    }),
  );

  await new Promise((resolve) => setImmediate(resolve));
  control.abort(new Error("cancel portable search"));
  await assert.rejects(pending, /cancel portable search/);
});

test("a clipped search line never returns half an emoji", async () => {
  const query = "unicode-boundary-token";
  const emoji = String.fromCodePoint(0x1f600);
  const line = `${query}${"x".repeat(498 - query.length)}${emoji}tail`;
  await fs.writeFile(path.join(ctx.root, "unicode-boundary.txt"), line, "utf8");

  const result = await searchText.run({ query, pattern: "*.txt" }, ctx);

  assert.equal(result.output.isWellFormed(), true);
  assert.equal(result.output.includes(emoji), false);
  assert.match(result.output, /…$/);
});

test("search skips binary files", async () => {
  const result = await searchText.run({ query: "needle", pattern: "*.dat" }, ctx);
  assert.equal(result.output, "[no matches]");
  assert.match(result.summary ?? "", /skipped 1/);
});

test("portable search rejects a candidate replaced after discovery", async () => {
  const area = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-search-replaced-"));
  const file = path.join(area, "candidate.txt");
  const parked = path.join(area, "candidate.old.txt");
  try {
    await fs.writeFile(file, "ordinary text\n", "utf8");
    const details = await fs.lstat(file, { bigint: true });
    await fs.rename(file, parked);
    await fs.writeFile(file, "outside needle\n", "utf8");

    const result = await portableSearch(
      [{ path: file, bytes: Number(details.size), expected: stableFileExpectation(details) }],
      { root: area },
      "needle",
      true,
      10,
    );

    assert.deepEqual(result, { matches: [], skipped: 1 });
  } finally {
    await fs.rm(area, { recursive: true, force: true });
  }
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
  const tools = builtinTools();
  const names = tools.map((tool) => tool.name);
  assert.ok(names.includes("find_files"));
  assert.ok(names.includes("search_text"));
  assert.deepEqual(
    tools.map((tool) => [tool.name, tool.concurrency]),
    [
      ["read_file", "shared"],
      ["list_dir", "shared"],
      ["find_files", "shared"],
      ["search_text", "shared"],
      ["edit_file", "exclusive"],
      ["write_file", "exclusive"],
      ["run_command", "exclusive"],
    ],
  );
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
