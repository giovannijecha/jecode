// Manual performance probe for the incremental transcript renderer.

import { performance } from "node:perf_hooks";
import type { Block } from "../../src/tui/blocks.ts";
import type { TranscriptViewport } from "../../src/tui/transcript-view.ts";
import { transcriptRenderer } from "../../src/tui/transcript-view.ts";
import { STEEL } from "../../src/ui/theme.ts";
import { reportBenchmark, round } from "./report.ts";

const blockCount = 20_000;
const stableFrames = 500;
const streamingFrames = 200;
const reasoningChars = 200_000;
const blocks: Block[] = Array.from(
  { length: blockCount },
  (_, index) => ({ kind: "answer", text: `answer ${index}: one concise line` }),
);
const live: Block = {
  kind: "reasoning",
  text: "working ".repeat(Math.ceil(reasoningChars / 8)).slice(0, reasoningChars),
  live: true,
};
blocks.push(live);

const transcript = transcriptRenderer();
const coldStart = firstViewport(100);
const coldReflow = settle(coldStart.viewport, 100);
const narrowStart = firstViewport(80);
const narrowReflow = settle(narrowStart.viewport, 80);
const wideStart = firstViewport(120);
const wideReflow = settle(wideStart.viewport, 120);
const cachedResize = measure(() => {
  for (let frame = 0; frame < 100; frame++) {
    transcript.viewport(blocks, frame % 2 === 0 ? 80 : 120, 40, 0, STEEL);
  }
});
const stable = measure(() => {
  for (let frame = 0; frame < stableFrames; frame++) {
    transcript.viewport(blocks, 120, 40, frame % 80, STEEL);
  }
});
const streaming = measure(() => {
  for (let frame = 0; frame < streamingFrames; frame++) {
    live.text += ` ${frame}`;
    transcript.invalidate(live);
    transcript.viewport(blocks, 120, 40, 0, STEEL);
  }
});
const sealed = measure(() => {
  live.live = false;
  transcript.invalidate(live);
  transcript.viewport(blocks, 120, 40, 0, STEEL);
});
const expandedLive = measure(() => {
  live.live = true;
  live.expanded = true;
  for (let frame = 0; frame < streamingFrames; frame++) {
    live.text += ` expanded-${frame}`;
    transcript.invalidate(live);
    transcript.viewport(blocks, 120, 40, 0, STEEL);
  }
});

reportBenchmark("incremental-transcript", {
  blocks: blocks.length,
  liveReasoningCharacters: live.text.length,
  viewports: [
    { columns: 100, start: coldStart, reflow: coldReflow },
    { columns: 80, start: narrowStart, reflow: narrowReflow },
    { columns: 120, start: wideStart, reflow: wideReflow },
  ].map(({ columns, start, reflow }) => ({
    columns,
    firstViewportMilliseconds: round(start.milliseconds),
    backgroundReflowMilliseconds: round(reflow.milliseconds),
    backgroundReflowFrames: reflow.frames,
  })),
  cachedResize: { frames: 100, millisecondsPerFrame: round(cachedResize / 100) },
  stable: { frames: stableFrames, millisecondsPerFrame: round(stable / stableFrames) },
  streaming: { frames: streamingFrames, millisecondsPerFrame: round(streaming / streamingFrames) },
  sealMilliseconds: round(sealed),
  expandedLive: { frames: streamingFrames, millisecondsPerFrame: round(expandedLive / streamingFrames) },
});

function measure(run: () => void): number {
  const started = performance.now();
  run();
  return performance.now() - started;
}

function firstViewport(width: number): { viewport: TranscriptViewport; milliseconds: number } {
  const started = performance.now();
  const viewport = transcript.viewport(blocks, width, 40, 0, STEEL);
  return { viewport, milliseconds: performance.now() - started };
}

function settle(initial: TranscriptViewport, width: number): { milliseconds: number; frames: number } {
  let viewport = initial;
  let frames = 0;
  const milliseconds = measure(() => {
    while (viewport.pending) {
      viewport = transcript.viewport(blocks, width, 40, 0, STEEL);
      frames++;
    }
  });
  return { milliseconds, frames };
}
