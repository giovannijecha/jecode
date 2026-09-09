// Focused catalogue-cardinality drill-down with nine samples per case.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { performance } from "node:perf_hooks";
import * as path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { reportBenchmark, round } from "./report.ts";

const [checkout = fileURLToPath(new URL("../..", import.meta.url)), ...extra] = process.argv.slice(2);
if (extra.length > 0) throw new Error("usage: catalog.ts [checkout]");
const root = path.resolve(checkout);
const git = (args: string[]): string => execFileSync("git", args, {
  cwd: root, encoding: "utf8", windowsHide: true, maxBuffer: 1_048_576,
}).trim();
const commit = git(["rev-parse", "HEAD"]);
const dirtyBefore = git(["status", "--porcelain"]) !== "";
const { ConversationTree } = await import(pathToFileURL(path.join(root, "src/conversation.ts")).href) as
  typeof import("../../src/conversation.ts");
const { DurableSessionStore } = await import(pathToFileURL(path.join(root, "src/sessions/store.ts")).href) as
  typeof import("../../src/sessions/store.ts");
const ITERATIONS = 9;
const cases = [];
for (const [sessions, nodesPerSession] of [[12, 1], [12, 200], [50, 1], [200, 1]]) {
  const temporary = await mkdtemp(path.join(tmpdir(), "jecode-catalog-scaling-"));
  try {
    const workspace = path.join(temporary, "workspace");
    await mkdir(workspace);
    const store = await DurableSessionStore.open(workspace, path.join(temporary, "sessions"));
    const conversation = ConversationTree.restore(Array.from({ length: nodesPerSession }, (_, index) => ({
      id: index + 1, parentId: index, revision: 1, createdAt: "2026-09-07T00:00:00.000Z",
      settlement: "completed", identity: { providerId: "ollama", model: "fixture", effort: "high" },
      messages: [
        { role: "user", content: [{ kind: "text", text: `question ${index}` }] },
        { role: "assistant", content: [{ kind: "text", text: `answer ${index}` }] },
      ], blocks: [],
    })), nodesPerSession);
    for (let index = 0; index < sessions; index++) await store.publish(conversation);
    await store.list(64);
    const timings = [];
    for (let index = 0; index < ITERATIONS; index++) {
      const started = performance.now();
      const entries = await store.list(64);
      timings.push(performance.now() - started);
      assert.equal(entries.length, Math.min(64, sessions));
      assert.ok(entries.every((entry) => entry.turns === nodesPerSession && !entry.active));
    }
    cases.push({ sessions, nodesPerSession,
      medianMilliseconds: round(timings.toSorted((a, b) => a - b)[Math.floor(ITERATIONS / 2)]),
      minimumMilliseconds: round(Math.min(...timings)), maximumMilliseconds: round(Math.max(...timings)) });
  } finally { await rm(temporary, { recursive: true, force: true }); }
}
reportBenchmark("session-catalog-scaling", {
  commit, dirty: dirtyBefore || git(["status", "--porcelain"]) !== "" || git(["rev-parse", "HEAD"]) !== commit,
  iterations: ITERATIONS, cases,
});
