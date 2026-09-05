// Run the real input loop, frame scheduler, composition, and differential painter.

import { performance } from "node:perf_hooks";
import { runApp } from "../../src/tui/app.ts";
import type { Session } from "../../src/session.ts";
import type { Cursor } from "../../src/tui/frame.ts";
import { painter } from "../../src/tui/frame.ts";
import type { Size } from "../../src/tui/screen.ts";
import { measuredOutput } from "./tui-output.ts";

export type Sample = {
  milliseconds: number;
  writeMilliseconds?: number;
  acknowledgedMilliseconds?: number;
  paintMilliseconds: number;
  bytes: number;
};
type Match = (text: string, cursor: Cursor | undefined) => boolean;
type Pending = {
  started: number;
  matches: Match;
  resolve(sample: Sample): void;
  reject(error: unknown): void;
};

export async function tuiHost(session: Session, initialSize: Size, bytesPerSecond?: number) {
  const output = measuredOutput(bytesPerSecond);
  const shutdown = new AbortController();
  let size = initialSize;
  let feed: ((chunk: string) => void) | undefined;
  let resize: (() => void) | undefined;
  let pending: Pending | undefined;
  let last = "";
  let current: { started: number; sample: Sample } | undefined;
  let failure: { error: unknown } | undefined;
  const painted = painter({ ready: output.ready, onReady: output.onReady, write(text) {
    const observed = current;
    if (observed !== undefined) observed.sample.writeMilliseconds = performance.now() - observed.started;
    const bytes = output.write(text, () => {
      if (observed !== undefined) {
        observed.sample.acknowledgedMilliseconds = performance.now() - observed.started;
      }
    });
    if (observed !== undefined) observed.sample.bytes = bytes;
    current = undefined;
  } });
  let ready = (): void => {};
  const first = new Promise<void>((resolve) => { ready = resolve; });
  const started = performance.now();
  let firstFrameMilliseconds = 0;
  const running = runApp(session, session.config.root, {
    shutdownSignal: shutdown.signal,
    screen: {
      size: () => size,
      enter() {}, leave() {}, setReducedMotion() {},
      onInput(handler) { feed = handler; return () => { feed = undefined; }; },
      onResize(handler) { resize = handler; return () => { resize = undefined; }; },
    },
    paint: {
      onReady: painted.onReady,
      invalidate: () => painted.invalidate(),
      paint(rows, cursor) {
        last = rows.join("\n").replace(/\u001b\[[0-9;]*m/gu, "");
        const matched = pending !== undefined && pending.matches(last, cursor) ? pending : undefined;
        const sample: Sample = { milliseconds: 0, paintMilliseconds: 0, bytes: 0 };
        if (matched !== undefined) current = { started: matched.started, sample };
        const paintStart = performance.now();
        painted.paint(rows, cursor);
        sample.paintMilliseconds = performance.now() - paintStart;
        if (matched !== undefined) {
          pending = undefined;
          sample.milliseconds = performance.now() - matched.started;
          matched.resolve(sample);
        }
        if (firstFrameMilliseconds === 0) {
          firstFrameMilliseconds = performance.now() - started;
          ready();
        }
      },
    },
  }).catch((error: unknown) => {
    failure = { error };
    pending?.reject(error);
    ready();
  });
  await first;
  if (failure !== undefined) { output.destroy(); throw failure.error; }

  return {
    firstFrameMilliseconds,
    text: () => last,
    input(chunk: string) {
      if (feed === undefined) throw new Error("benchmark input is closed");
      feed(chunk);
    },
    resize(next: Size) { size = next; resize?.(); },
    async measure(action: () => void, matches: Match): Promise<Sample> {
      if (pending !== undefined) throw new Error("a benchmark action is already pending");
      if (failure !== undefined) throw failure.error;
      let timer: NodeJS.Timeout | undefined;
      try {
        return await new Promise<Sample>((resolve, reject) => {
          pending = { started: performance.now(), matches, resolve, reject };
          timer = setTimeout(() => reject(new Error("benchmark frame did not show the expected change")), 5_000);
          action();
        });
      } finally {
        clearTimeout(timer);
        pending = undefined;
      }
    },
    stats: output.stats,
    async close(drain = true) {
      pending?.reject(new Error("benchmark host closed"));
      current = undefined;
      shutdown.abort();
      await running;
      if (drain && failure === undefined) await output.close();
      else output.destroy();
      if (failure !== undefined) throw failure.error;
    },
  };
}
