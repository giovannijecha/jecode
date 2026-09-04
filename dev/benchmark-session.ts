// Manual regression probe for incremental durable session checkpoints.

import { performance } from "node:perf_hooks";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { ConversationTree } from "../src/conversation.ts";
import type { TurnNode } from "../src/conversation.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";

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
const identity = {
  providerId: "ollama",
  model: "deepseek-v4-flash:0731",
  effort: "high",
};

const small = await sampleCheckpoint(SMALL_NODES);
const large = await sampleCheckpoint(LARGE_NODES);
const scale = large / small;
const checkpointPassed = large <= MAX_LARGE_MEDIAN_MS && scale <= MAX_SCALE;
const shallowCatalog = await sampleCatalog(CATALOG_SMALL_NODES);
const deepCatalog = await sampleCatalog(CATALOG_LARGE_NODES);
const catalogDepthDelta = deepCatalog - shallowCatalog;
const catalogPassed = deepCatalog <= MAX_CATALOG_MEDIAN_MS &&
  catalogDepthDelta <= MAX_CATALOG_DEPTH_DELTA_MS;
const passed = checkpointPassed && catalogPassed;

process.stdout.write(`${JSON.stringify({
  benchmark: "durable-session-store",
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
  passed,
})}\n`);

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
    return median(timings);
  } finally {
    await persistence?.close();
    await rm(root, { recursive: true, force: true });
  }
}

async function sampleCatalog(nodeCount: number): Promise<number> {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-session-catalog-bench-"));
  const workspace = path.join(root, "workspace");
  const sessions = path.join(root, "sessions");
  await mkdir(workspace);
  try {
    const store = await DurableSessionStore.open(workspace, sessions);
    const conversation = ConversationTree.restore(nodes(nodeCount), nodeCount);
    for (let index = 0; index < CATALOG_SESSIONS; index++) {
      await store.publish(conversation);
    }
    await store.list();
    const timings: number[] = [];
    for (let iteration = 0; iteration < ITERATIONS; iteration++) {
      const startedAt = performance.now();
      await store.list();
      timings.push(performance.now() - startedAt);
    }
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

function round(value: number): number {
  return Number(value.toFixed(3));
}
