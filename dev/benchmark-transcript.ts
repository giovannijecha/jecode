// Manual performance probe for the incremental transcript renderer.

import { performance } from "node:perf_hooks";
import type { Block } from "../src/tui/blocks.ts";
import { transcriptRenderer } from "../src/tui/transcript-view.ts";
import { STEEL } from "../src/ui/theme.ts";

const blockCount = 20_000;
const stableFrames = 500;
const streamingFrames = 200;
const blocks: Block[] = Array.from(
  { length: blockCount },
  (_, index) => ({ kind: "answer", text: `answer ${index}: one concise line` }),
);
const live: Block = { kind: "reasoning", text: "working", live: true };
blocks.push(live);

const transcript = transcriptRenderer();
const coldStart = measure(() => transcript.viewport(blocks, 100, 40, 0, STEEL));
const stable = measure(() => {
  for (let frame = 0; frame < stableFrames; frame++) {
    transcript.viewport(blocks, 100, 40, frame % 80, STEEL);
  }
});
const streaming = measure(() => {
  for (let frame = 0; frame < streamingFrames; frame++) {
    live.text += ` ${frame}`;
    transcript.invalidate(live);
    transcript.viewport(blocks, 100, 40, 0, STEEL);
  }
});

process.stdout.write([
  `blocks: ${blocks.length.toLocaleString()}`,
  `cold layout: ${coldStart.toFixed(2)} ms`,
  `stable viewport: ${(stable / stableFrames).toFixed(3)} ms/frame`,
  `streaming tail: ${(streaming / streamingFrames).toFixed(3)} ms/frame`,
].join("\n") + "\n");

function measure(run: () => void): number {
  const started = performance.now();
  run();
  return performance.now() - started;
}
