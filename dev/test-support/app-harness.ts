import type { Painter } from "../../src/tui/frame.ts";
import type { AppScreen } from "../../src/tui/app.ts";

export function virtualScreen(cols = 70): {
  environment: { screen: AppScreen; paint: Painter };
  frames: string[][];
  input(): Promise<(chunk: string) => void>;
  left(): boolean;
} {
  let feed: ((chunk: string) => void) | undefined;
  let left = false;
  const frames: string[][] = [];
  const screen: AppScreen = {
    size: () => ({ rows: 18, cols }),
    enter: () => {},
    leave: () => {
      left = true;
    },
    setReducedMotion: () => {},
    onResize: () => () => {},
    onInput: (handler) => {
      feed = handler;
      return () => {};
    },
  };
  const paint: Painter = {
    paint: (rows) => frames.push([...rows]),
    invalidate: () => {},
  };
  return {
    environment: { screen, paint },
    frames,
    async input() {
      await waitFor(() => feed !== undefined, "TUI input handler");
      return feed as (chunk: string) => void;
    },
    left: () => left,
  };
}

export async function waitFor(
  condition: () => boolean | Promise<boolean>,
  label: string,
  timeoutMs = 2_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!(await condition())) {
    if (Date.now() >= deadline) throw new Error(`timed out waiting for ${label}`);
    await new Promise<void>((resolve) => setTimeout(resolve, 5));
  }
}

export function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export function plainRow(value: string): string {
  return value.replace(/\u001b\[[0-9;]*m/gu, "");
}

export function lastFooter(harness: { frames: string[][] }): string {
  return plainRow(harness.frames.at(-1)?.at(-1) ?? "");
}

export async function waitForIdle(harness: { frames: string[][] }, label: string): Promise<void> {
  await waitFor(() => !lastFooter(harness).includes("esc to interrupt"), label);
}

export async function waitForExportCompletion(
  harness: { frames: string[][] },
  startedAt: number,
  label: string,
): Promise<void> {
  await waitFor(
    () => harness.frames.slice(startedAt).some((frame) => frame.join("\n").includes("saved ·")),
    `${label} feedback`,
  );
  await waitForIdle(harness, `${label} idle state`);
}

export function deferred(): { wait: Promise<void>; release(): void } {
  let release = (): void => {};
  const wait = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { wait, release };
}
