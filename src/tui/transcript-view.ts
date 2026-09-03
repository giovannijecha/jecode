// Incremental transcript layout for long-running sessions.

import type { Palette } from "../ui/theme.ts";
import type { Block, Detail } from "./blocks.ts";
import type { RenderContext } from "./blocks.ts";
import { render } from "./blocks.ts";
import {
  TOOL_BIRTH_MS,
  TOOL_LEADER_MAX_MS,
  TOOL_ROW_ARRIVAL_MS,
  TOOL_SETTLE_MS,
} from "./motion.ts";

const MAX_LAYOUTS = 2;
const REFLOW_BLOCKS_PER_FRAME = 128;

type DrawBlock = (block: Block, width: number, palette: Palette, context?: RenderContext) => string[];
type RenderState = Omit<RenderContext, "motion" | "previous">;

type MotionRecord = {
  bornAt: number;
  settledAt?: number;
  rowsAt: number[];
  rowKeys: string[];
  tone: Extract<Block, { kind: "tool" }>["tone"];
};

type Entry = {
  block: Block;
  rows?: string[];
  rowCount: number;
};

type Layout = {
  source: readonly Block[];
  width: number;
  palette: Palette;
  entries: Entry[];
  ends: number[];
  positions: WeakMap<Block, number>;
  pending: number;
  next: number;
};

type Window = {
  first: number;
  last: number;
  start: number;
  end: number;
  maxScroll: number;
};

export type TranscriptViewport = {
  rows: string[];
  maxScroll: number;
  pending: boolean;
  animating: boolean;
};

export type TranscriptRenderer = {
  viewport(
    blocks: readonly Block[],
    width: number,
    height: number,
    scroll: number,
    palette: Palette,
    state?: RenderState,
  ): TranscriptViewport;
  /** Reflow everything, or only one block whose semantic state changed. */
  invalidate(block?: Block): void;
};

/**
 * Keep recent widths and reflow a bounded working set on each frame.
 *
 * The visible window is always rendered before it is returned. Older rows use
 * their last known height only to guide scrolling while the remaining blocks
 * are refreshed over later frames. This keeps resize responsive without a
 * worker, hidden loop, or unbounded multi-width cache.
 */
export function transcriptRenderer(
  draw: DrawBlock = render,
  clock: () => number = Date.now,
): TranscriptRenderer {
  let layouts: Layout[] = [];
  const motions = new WeakMap<Block, MotionRecord>();
  const activeMotion = new Set<Block>();

  return {
    viewport(blocks, width, height, scroll, palette, state = {}) {
      const layout = layoutFor(blocks, width, palette);
      const now = state.now ?? clock();
      if (scroll === 0 && state.reducedMotion !== true) {
        invalidateMovingEntries(layout, now);
      }
      // Reflowing rows below a scroll-locked viewport would move the content
      // being read as estimates become exact. Visible rows still resolve on
      // demand; background work resumes when the user follows the tail again.
      if (scroll === 0) reflowBatch(layout, state);
      const window = reflowWindow(layout, Math.max(0, height), scroll, state);
      return {
        rows: visibleRows(layout, window, Math.max(0, height)),
        maxScroll: window.maxScroll,
        pending: scroll === 0 && layout.pending > 0,
        animating: scroll === 0 && state.reducedMotion !== true && hasMovingEntry(layout, now),
      };
    },

    invalidate(block) {
      if (block === undefined) {
        layouts = [];
        return;
      }
      observeMotion(block, clock());
      for (const layout of layouts) {
        const index = layout.positions.get(block);
        if (index === undefined) continue;
        dirty(layout, index);
      }
    },
  };

  function layoutFor(blocks: readonly Block[], width: number, palette: Palette): Layout {
    const cachedAt = layouts.findIndex((layout) =>
      layout.width === width && layout.palette === palette
    );
    if (cachedAt !== -1) {
      const cached = layouts[cachedAt] as Layout;
      layouts.splice(cachedAt, 1);
      if (compatible(cached, blocks)) {
        append(cached, blocks);
        layouts.push(cached);
        return cached;
      }
    }

    const seed = [...layouts].reverse().find((layout) => compatiblePrefix(layout, blocks));
    const created = createLayout(blocks, width, palette, seed);
    layouts.push(created);
    if (layouts.length > MAX_LAYOUTS) layouts.shift();
    return created;
  }

  function createLayout(
    blocks: readonly Block[],
    width: number,
    palette: Palette,
    seed: Layout | undefined,
  ): Layout {
    const layout: Layout = {
      source: blocks,
      width,
      palette,
      entries: [],
      ends: [],
      positions: new WeakMap<Block, number>(),
      pending: 0,
      next: -1,
    };
    append(layout, blocks, seed);
    return layout;
  }

  function append(layout: Layout, blocks: readonly Block[], seed?: Layout): void {
    const firstAdded = layout.entries.length;
    let total = layout.ends.at(-1) ?? 0;
    for (let index = firstAdded; index < blocks.length; index++) {
      const block = blocks[index];
      if (block === undefined) continue;
      const seeded = seed?.entries[index];
      const rowCount = seeded?.block === block ? seeded.rowCount : estimatedRows(block);
      layout.entries.push({ block, rowCount });
      layout.positions.set(block, index);
      layout.pending++;
      total += rowCount;
      layout.ends.push(total);
    }
    if (layout.entries.length > firstAdded) {
      layout.next = Math.max(layout.next, layout.entries.length - 1);
    }
  }

  function reflowBatch(layout: Layout, state: RenderState): void {
    let remaining = REFLOW_BLOCKS_PER_FRAME;
    let firstChanged = layout.entries.length;
    let index = layout.next;
    while (index >= 0 && remaining > 0 && layout.pending > 0) {
      if (renderEntry(layout, index, state)) {
        firstChanged = Math.min(firstChanged, index);
        remaining--;
      }
      index--;
    }
    layout.next = index;
    if (firstChanged < layout.entries.length) recomputeEnds(layout, firstChanged);
  }

  function reflowWindow(
    layout: Layout,
    height: number,
    scroll: number,
    state: RenderState,
  ): Window {
    while (true) {
      const window = visibleWindow(layout, height, scroll);
      let firstChanged = layout.entries.length;
      for (let index = window.first; index <= window.last; index++) {
        if (renderEntry(layout, index, state)) firstChanged = Math.min(firstChanged, index);
      }
      if (firstChanged === layout.entries.length) return window;
      recomputeEnds(layout, firstChanged);
    }
  }

  function renderEntry(layout: Layout, index: number, state: RenderState): boolean {
    const entry = layout.entries[index];
    if (entry === undefined || entry.rows !== undefined) return false;
    entry.rows = draw(entry.block, layout.width, layout.palette, {
      ...state,
      previous: layout.entries[index - 1]?.block,
      motion: motions.get(entry.block),
    });
    entry.rowCount = entry.rows.length;
    layout.pending--;
    return true;
  }

  function recomputeEnds(layout: Layout, from: number): void {
    let total = from === 0 ? 0 : (layout.ends[from - 1] ?? 0);
    for (let index = from; index < layout.entries.length; index++) {
      total += layout.entries[index]?.rowCount ?? 0;
      layout.ends[index] = total;
    }
  }

  function visibleWindow(layout: Layout, height: number, scroll: number): Window {
    const totalRows = layout.ends.at(-1) ?? 0;
    const maxScroll = Math.max(0, totalRows - height);
    const offset = Math.min(Math.max(0, scroll), maxScroll);
    const start = Math.max(0, totalRows - height - offset);
    const end = start + height;
    const first = firstEndingAfter(layout.ends, start);
    let last = first - 1;
    let at = first === 0 ? 0 : (layout.ends[first - 1] ?? 0);
    while (last + 1 < layout.entries.length && at < end) {
      last++;
      at = layout.ends[last] ?? at;
    }
    return { first, last, start, end, maxScroll };
  }

  function visibleRows(layout: Layout, window: Window, height: number): string[] {
    const rows: string[] = [];
    let index = window.first;
    let at = index === 0 ? 0 : (layout.ends[index - 1] ?? 0);
    while (index <= window.last && at < window.end) {
      const blockRows = layout.entries[index]?.rows ?? [];
      rows.push(...blockRows.slice(
        Math.max(0, window.start - at),
        Math.min(blockRows.length, window.end - at),
      ));
      at = layout.ends[index] ?? at;
      index++;
    }
    while (rows.length < height) rows.unshift("");
    return rows;
  }

  function dirty(layout: Layout, index: number): void {
    const entry = layout.entries[index];
    if (entry?.rows !== undefined) {
      entry.rows = undefined;
      layout.pending++;
    }
    layout.next = Math.max(layout.next, index);
  }

  function observeMotion(block: Block, now: number): void {
    if (block.kind !== "tool") return;
    let motion = motions.get(block);
    // A settled historical block may be invalidated by Ctrl+O. It should not
    // replay its original arrival animation after resume.
    if (motion === undefined) {
      if (block.tone !== "pending") return;
      motion = {
        bornAt: now,
        rowsAt: (block.body ?? []).map(() => now),
        rowKeys: (block.body ?? []).map(detailKey),
        tone: block.tone,
      };
      motions.set(block, motion);
      activeMotion.add(block);
      return;
    }

    if (motion.tone === "pending" && block.tone !== "pending") motion.settledAt = now;
    motion.tone = block.tone;
    reconcileRows(motion, block.body ?? [], now);
    activeMotion.add(block);
  }

  function invalidateMovingEntries(layout: Layout, now: number): void {
    for (const block of activeMotion) {
      const index = layout.positions.get(block);
      if (index === undefined) {
        activeMotion.delete(block);
        continue;
      }
      const motion = motions.get(block);
      if (motion === undefined || !moving(block, motion, now)) {
        // Paint the resting state once before leaving the motion registry;
        // otherwise the cache could retain the final in-between frame.
        dirty(layout, index);
        activeMotion.delete(block);
        continue;
      }
      dirty(layout, index);
    }
  }

  function hasMovingEntry(layout: Layout, now: number): boolean {
    for (const block of activeMotion) {
      if (layout.positions.get(block) === undefined) continue;
      const motion = motions.get(block);
      if (motion !== undefined && moving(block, motion, now)) return true;
    }
    return false;
  }
}

function reconcileRows(motion: MotionRecord, details: readonly Detail[], now: number): void {
  const available = new Map<string, { values: number[]; next: number }>();
  for (let index = 0; index < motion.rowKeys.length; index++) {
    const key = motion.rowKeys[index];
    if (key === undefined) continue;
    const bucket = available.get(key) ?? { values: [], next: 0 };
    bucket.values.push(motion.rowsAt[index] ?? motion.bornAt);
    available.set(key, bucket);
  }

  const rowKeys = details.map(detailKey);
  const rowsAt = rowKeys.map((key) => {
    const bucket = available.get(key);
    if (bucket === undefined || bucket.next >= bucket.values.length) return now;
    const value = bucket.values[bucket.next] ?? now;
    bucket.next++;
    return value;
  });
  motion.rowKeys = rowKeys;
  motion.rowsAt = rowsAt;
}

function detailKey(detail: Detail): string {
  if (detail.kind === "out" || detail.kind === "gap") {
    return JSON.stringify([detail.kind, detail.text]);
  }
  return JSON.stringify([
    detail.kind,
    detail.oldLine ?? "",
    detail.newLine ?? "",
    detail.emphasis?.start ?? "",
    detail.emphasis?.length ?? "",
    detail.text,
  ]);
}

function moving(block: Block, motion: MotionRecord, now: number): boolean {
  if (
    block.kind === "tool" && block.tone === "pending" &&
    block.startedAt !== undefined && block.right === "running"
  ) return true;

  let until = motion.bornAt + Math.max(TOOL_BIRTH_MS, TOOL_LEADER_MAX_MS);
  if (motion.settledAt !== undefined) until = Math.max(until, motion.settledAt + TOOL_SETTLE_MS);
  for (const arrivedAt of motion.rowsAt) {
    until = Math.max(until, arrivedAt + TOOL_ROW_ARRIVAL_MS);
  }
  return now < until;
}

function compatible(layout: Layout, blocks: readonly Block[]): boolean {
  return layout.source === blocks &&
    blocks.length >= layout.entries.length &&
    !edgeChanged(layout, blocks);
}

function compatiblePrefix(layout: Layout, blocks: readonly Block[]): boolean {
  if (layout.entries.length === 0 || blocks.length === 0) return true;
  const shared = Math.min(layout.entries.length, blocks.length);
  return layout.entries[0]?.block === blocks[0] &&
    layout.entries[shared - 1]?.block === blocks[shared - 1];
}

function edgeChanged(layout: Layout, blocks: readonly Block[]): boolean {
  if (layout.entries.length === 0) return false;
  const lastShared = Math.min(blocks.length, layout.entries.length) - 1;
  return lastShared < 0 ||
    layout.entries[0]?.block !== blocks[0] ||
    layout.entries[lastShared]?.block !== blocks[lastShared];
}

function estimatedRows(block: Block): number {
  return block.kind === "user" ? 4 : 2;
}

function firstEndingAfter(ends: readonly number[], row: number): number {
  let low = 0;
  let high = ends.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if ((ends[middle] ?? 0) <= row) low = middle + 1;
    else high = middle;
  }
  return low;
}
