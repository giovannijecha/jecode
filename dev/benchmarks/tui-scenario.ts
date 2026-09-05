// Synthetic history and streaming provider; no network, tools, or persistence.

import { ConversationTree } from "../../src/conversation.ts";
import type { SendRequest } from "../../src/types.ts";
import type { Block } from "../../src/tui/blocks.ts";
import { session, provider } from "../test-support/app.ts";
import { aborted } from "../test-support/controller.ts";
import { tuiHost } from "./tui-host.ts";
import type { Sample } from "./tui-host.ts";
import { round } from "./report.ts";

const ESC = "\u001b";
const CLEAR = "\u0015";
const WARMUP = 25;
const SAMPLES = 100;

export async function scenario(columns: number, blocks: number, bytesPerSecond?: number) {
  const memoryBefore = memory();
  let request: SendRequest | undefined;
  let requests = 0;
  let ready = (): void => {};
  const requestReady = new Promise<void>((resolve) => { ready = resolve; });
  const current = session({ ...provider(), send(next) {
    requests++;
    request = next;
    ready();
    return aborted(next.signal);
  } });
  current.config.ephemeral = true;
  current.conversation = history(blocks);
  const host = await tuiHost(current, { cols: columns, rows: 40 }, bytesPerSecond);
  const results: Record<string, Sample[]> = {};
  let complete = false;
  try {
    for (let i = 0; i < WARMUP; i++) await type(`warm-${i}`);
    const memoryLoaded = memory();
    results["typing"] = await collect((i) => type(`idle-${i}`));
    host.input(`${CLEAR}work\r`);
    await readyWithin(requestReady);
    request?.onStream?.({ kind: "thinking", text: "bounded reasoning ".repeat(12_500) });
    results["typingWhileStreaming"] = await collect((i) => {
      stream(i);
      return type(`live-${i}`);
    });
    await host.measure(() => host.input(`${CLEAR}/settings\r`), (text) => text.includes("● model"));
    results["menuWhileStreaming"] = await collect((i) => {
      const down = i % 2 === 0;
      return host.measure(() => {
        stream(i);
        host.input(`${ESC}[${down ? "B" : "A"}`);
      }, (text) => text.includes(down ? "● effort" : "● model"));
    });
    await host.measure(() => host.input(ESC), (text, cursor) => cursor !== undefined && !text.includes("● model"));
    results["resizeWhileStreaming"] = await collect((i) => {
      const width = i % 2 === 0 ? (columns === 60 ? 120 : 60) : columns;
      return host.measure(() => {
        stream(i);
        host.resize({ cols: width, rows: 40 });
        host.input(`${CLEAR}resize-${i}`);
      }, (text, cursor) => cursor !== undefined && text.includes(`resize-${i}`));
    });
    await host.measure(() => host.input(ESC), (_text, cursor) =>
      cursor !== undefined && current.conversation.activeNode?.settlement === "interrupted");
    const memoryAfterWork = memory();
    await host.measure(() => host.input(`${CLEAR}/new\r`), (_text, cursor) =>
      cursor !== undefined && current.conversation.nodes.length === 0);
    const memoryAfterReset = memory();
    if (requests !== 1) throw new Error(`expected one inert provider request, received ${requests}`);
    // Drain before reading acknowledgement samples; enqueue latency alone hides slow output.
    await host.close();
    complete = true;
    return {
      columns, rows: 40, historyBlocks: blocks, warmup: WARMUP,
      firstFrameMilliseconds: round(host.firstFrameMilliseconds),
      outputBytesPerSecond: bytesPerSecond ?? null,
      scenarios: Object.fromEntries(Object.entries(results).map(([name, samples]) => [name, summarize(samples)])),
      output: host.stats(),
      memory: { before: memoryBefore, loaded: memoryLoaded, afterWork: memoryAfterWork,
        afterReset: memoryAfterReset, afterClose: memory() },
    };
  } finally {
    request = undefined;
    if (!complete) await host.close(false);
  }

  function type(text: string): Promise<Sample> {
    return host.measure(() => host.input(`${CLEAR}${text}`), (frame, cursor) =>
      cursor !== undefined && frame.includes(text));
  }

  function stream(index: number): void {
    request?.onStream?.({ kind: "text", text: `stream ${index}: measured fixture output\n` });
  }
}

function history(count: number): ConversationTree {
  if (count === 0) return ConversationTree.empty();
  const blocks: Block[] = Array.from({ length: count - 1 }, (_, i) => ({
    kind: "answer", text: `History ${i}: stable transcript evidence for this fixture.`,
  }));
  blocks.push({ kind: "tool", name: "run_command", target: "inert fixture", right: "2,000 lines",
    tone: "ok", durationMs: 25, body: Array.from({ length: 2_000 }, (_, i) => ({
      kind: "out", text: `fixture output ${i}`,
    })),
  });
  return ConversationTree.empty().commit({ parentId: 0,
    createdAt: "2026-01-01T00:00:00.000Z", identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [{ role: "user", content: [{ kind: "text", text: "fixture" }] },
      { role: "assistant", content: [{ kind: "text", text: "fixture answer" }] }], blocks,
  }, "completed");
}

async function collect(action: (index: number) => Promise<Sample>): Promise<Sample[]> {
  const result: Sample[] = [];
  for (let i = 0; i < SAMPLES; i++) result.push(await action(i));
  return result;
}

function summarize(samples: Sample[]) {
  const written = samples.filter((sample) => sample.writeMilliseconds !== undefined);
  return {
    samples: samples.length,
    written: written.length,
    coalesced: samples.length - written.length,
    inputToFrameMilliseconds: distribution(samples.map((sample) => sample.milliseconds)),
    inputToWriteMilliseconds: distribution(written.map((sample) => sample.writeMilliseconds as number)),
    inputToAcknowledgementMilliseconds: distribution(written.map((sample) => {
      if (sample.acknowledgedMilliseconds === undefined) throw new Error("output acknowledgement missing");
      return sample.acknowledgedMilliseconds;
    })),
    painterMilliseconds: distribution(samples.map((sample) => sample.paintMilliseconds)),
    bytesPerWrittenFrame: distribution(written.map((sample) => sample.bytes)),
  };
}

function distribution(values: number[]) {
  if (values.length === 0) return null;
  const sorted = values.toSorted((left, right) => left - right);
  const n = sorted.length;
  return { median: round(((sorted[Math.floor((n - 1) / 2)] ?? 0) + (sorted[Math.floor(n / 2)] ?? 0)) / 2),
    p95: round(sorted[Math.ceil(n * 0.95) - 1] ?? 0), max: round(sorted.at(-1) ?? 0) };
}

function memory() {
  if (global.gc === undefined) throw new Error("run the TUI probe with --expose-gc");
  global.gc();
  const { rss, heapUsed, external, arrayBuffers } = process.memoryUsage();
  return { rss, heapUsed, external, arrayBuffers };
}

async function readyWithin(ready: Promise<void>): Promise<void> {
  let timer: NodeJS.Timeout | undefined;
  try {
    await Promise.race([ready, new Promise<never>((_resolve, reject) => {
      timer = setTimeout(() => reject(new Error("synthetic provider did not start")), 5_000);
    })]);
  } finally {
    clearTimeout(timer);
  }
}
