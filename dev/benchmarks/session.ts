// Manual regression probe for incremental durable session checkpoints.

import { performance } from "node:perf_hooks";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../../src/conversation.ts";
import type { TurnNode } from "../../src/conversation.ts";
import { DurableSessionStore } from "../../src/sessions/store.ts";
import { SessionPersistence } from "../../src/sessions/runtime.ts";
import { reportBenchmark, round } from "./report.ts";
import { distribution } from "./samples.ts";

const ITERATIONS = 5;
const SMALL_NODES = 50;
const LARGE_NODES = 750;
const MAX_LARGE_MEDIAN_MS = 25;
const MAX_SCALE = 2;
const CATALOG_SESSIONS = 12;
const CATALOG_SMALL_NODES = 1;
const CATALOG_LARGE_NODES = 200;
const MAX_CATALOG_MEDIAN_MS = 20;
const MAX_CATALOG_DEPTH_DELTA_MS = 10;
const LOAD_ITERATIONS = 3;
const LOAD_NODE_COUNTS = [1, 200, 750, 1_024] as const;
const MAX_LOAD_MEDIAN_MS = 350;
const MAX_LOAD_SCALE = 7;
const identity = {
  providerId: "ollama",
  model: "deepseek-v4-flash:0731",
  effort: "high",
};
const observations: Record<string, ReturnType<typeof distribution>> = {};

const small = await sampleCheckpoint(SMALL_NODES);
const large = await sampleCheckpoint(LARGE_NODES);
const scale = large / small;
const checkpointPassed = large <= MAX_LARGE_MEDIAN_MS && scale <= MAX_SCALE;
const shallowCatalog = await sampleCatalog(CATALOG_SMALL_NODES);
const deepCatalog = await sampleCatalog(CATALOG_LARGE_NODES);
const cardinality = [];
for (const sessions of [50, 200]) {
  cardinality.push({ sessions, nodesPerSession: 1,
    medianMilliseconds: round(await sampleCatalog(1, sessions)) });
}
const catalogDepthDelta = deepCatalog - shallowCatalog;
const catalogPassed = deepCatalog <= MAX_CATALOG_MEDIAN_MS &&
  catalogDepthDelta <= MAX_CATALOG_DEPTH_DELTA_MS;
const loadResults: Array<{ nodes: number; medianMilliseconds: number }> = [];
for (const nodeCount of LOAD_NODE_COUNTS) {
  loadResults.push({ nodes: nodeCount, medianMilliseconds: await sampleLoad(nodeCount) });
}
const loadScale = (loadResults.at(-1)?.medianMilliseconds ?? Infinity) /
  (loadResults[1]?.medianMilliseconds ?? 1);
const loadPassed = (loadResults.at(-1)?.medianMilliseconds ?? Infinity) <=
    MAX_LOAD_MEDIAN_MS && loadScale <= MAX_LOAD_SCALE;
const passed = checkpointPassed && catalogPassed && loadPassed;

reportBenchmark("durable-session-store", {
  checkpoint: {
    iterations: ITERATIONS,
    results: [
      { nodes: SMALL_NODES, medianMilliseconds: round(small) },
      { nodes: LARGE_NODES, medianMilliseconds: round(large) },
    ],
    thresholds: {
      largeMedianMilliseconds: MAX_LARGE_MEDIAN_MS,
      maximumScale: MAX_SCALE,
    },
    observedScale: round(scale),
    passed: checkpointPassed,
  },
  catalog: {
    iterations: ITERATIONS,
    sessions: CATALOG_SESSIONS,
    results: [
      { nodesPerSession: CATALOG_SMALL_NODES, medianMilliseconds: round(shallowCatalog) },
      { nodesPerSession: CATALOG_LARGE_NODES, medianMilliseconds: round(deepCatalog) },
    ],
    thresholds: {
      deepMedianMilliseconds: MAX_CATALOG_MEDIAN_MS,
      maximumDepthDeltaMilliseconds: MAX_CATALOG_DEPTH_DELTA_MS,
    },
    observedDepthDeltaMilliseconds: round(catalogDepthDelta),
    passed: catalogPassed,
  },
  load: {
    iterations: LOAD_ITERATIONS,
    results: loadResults.map((result) => ({
      nodes: result.nodes,
      medianMilliseconds: round(result.medianMilliseconds),
    })),
    thresholds: {
      largestMedianMilliseconds: MAX_LOAD_MEDIAN_MS,
      maximumScaleFrom200To1024: MAX_LOAD_SCALE,
    },
    observedScaleFrom200To1024: round(loadScale),
    passed: loadPassed,
  },
  cardinality,
  observations,
  passed,
});

if (!passed) process.exitCode = 1;

async function sampleCheckpoint(nodeCount: number): Promise<number> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-session-bench-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  let persistence: SessionPersistence | undefined;
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    let conversation = ConversationTree.restore(nodes(nodeCount), nodeCount);
    persistence = SessionPersistence.fresh(store);
    await persistence.checkpoint(conversation);
    const timings: number[] = [];
    for (let iteration = 0; iteration <= ITERATIONS; iteration++) {
      const startedAt = performance.now();
      conversation = revise(conversation, iteration);
      await persistence.checkpoint(conversation);
      const elapsed = performance.now() - startedAt;
      if (iteration > 0) timings.push(elapsed);
    }
    assert.ok(persistence.sessionId, "checkpoint must publish a session");
    await persistence.close();
    const saved = await store.load(persistence.sessionId);
    assert.ok(saved, "checkpoint must be readable from the store");
    assert.equal(saved.meta.id, persistence.sessionId);
    assert.deepEqual(saved.conversation.nodes, JSON.parse(JSON.stringify(conversation.nodes)),
      "checkpoint must persist the final revision");
    assert.equal(saved.conversation.activeNodeId, conversation.activeNodeId);
    observations[`checkpoint-${nodeCount}`] = distribution(timings);
    return median(timings);
  } finally {
    await persistence?.close();
    await rm(root, { recursive: true, force: true });
  }
}

async function sampleCatalog(nodeCount: number, sessionCount = CATALOG_SESSIONS): Promise<number> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-session-catalog-bench-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const conversation = ConversationTree.restore(nodes(nodeCount), nodeCount);
    const expected = [];
    for (let index = 0; index < sessionCount; index++) {
      const published = await store.publish(conversation);
      expected.push({ id: published.meta.id, updatedAt: published.head.updatedAt });
    }
    const ids = expected.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt) || b.id.localeCompare(a.id))
      .slice(0, 64).map(entry => entry.id);
    await store.list(64);
    const timings: number[] = [];
    for (let iteration = 0; iteration < ITERATIONS; iteration++) {
      const startedAt = performance.now();
      const entries = await store.list(64);
      timings.push(performance.now() - startedAt);
      assert.deepEqual(entries.map(entry => entry.id), ids, "catalogue must return the newest sessions in order");
      assert.ok(entries.every(entry => entry.turns === nodeCount && !entry.active && entry.preview === "question 0"));
    }
    observations[`catalog-${sessionCount}-${nodeCount}`] = distribution(timings);
    return median(timings);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

async function sampleLoad(nodeCount: number): Promise<number> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-session-load-bench-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const expected = ConversationTree.restore(nodes(nodeCount), nodeCount);
    const published = await store.publish(expected);
    await store.load(published.meta.id);
    const timings: number[] = [];
    for (let iteration = 0; iteration < LOAD_ITERATIONS; iteration++) {
      const startedAt = performance.now();
      const loaded = await store.load(published.meta.id);
      timings.push(performance.now() - startedAt);
      assert.ok(loaded, "load must return the published session");
      assert.equal(loaded.meta.id, published.meta.id);
      assert.equal(loaded.conversation.activeNodeId, nodeCount);
      assert.deepEqual(loaded.conversation.nodes, expected.nodes, "load must return every complete node");
    }
    observations[`load-${nodeCount}`] = distribution(timings);
    return median(timings);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

function nodes(count: number): TurnNode[] {
  return Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    parentId: index,
    revision: 1,
    createdAt: "2026-09-03T12:00:00.000Z",
    settlement: "completed" as const,
    identity,
    messages: messages(`question ${index}`, `answer ${index}`),
    blocks: [],
  }));
}

function revise(conversation: ConversationTree, iteration: number): ConversationTree {
  const active = conversation.activeNode as TurnNode;
  return conversation.commit({
    nodeId: active.id,
    parentId: active.parentId,
    createdAt: active.createdAt,
    identity: active.identity,
    messages: messages(`revision ${iteration}`, "done"),
    blocks: [],
  }, "completed");
}

function messages(user: string, answer: string) {
  return [
    { role: "user" as const, content: [{ kind: "text" as const, text: user }] },
    { role: "assistant" as const, content: [{ kind: "text" as const, text: answer }] },
  ];
}

function median(values: readonly number[]): number {
  return [...values].sort((left, right) => left - right)[Math.floor(values.length / 2)] as number;
}
