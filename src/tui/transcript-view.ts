// Incremental transcript layout for long-running sessions.

import type { Palette } from "../ui/theme.ts";
import type { Block } from "./blocks.ts";
import type { RenderContext } from "./blocks.ts";
import { render } from "./blocks.ts";

const MAX_LAYOUTS = 2;
const REFLOW_BLOCKS_PER_FRAME = 128;

type DrawBlock = (block: Block, width: number, palette: Palette, context?: RenderContext) => string[];
type RenderState = Omit<RenderContext, "previous">;

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
export function transcriptRenderer(draw: DrawBlock = render): TranscriptRenderer {
  let layouts: Layout[] = [];

  return {
    viewport(blocks, width, height, scroll, palette, state = {}) {
      const layout = layoutFor(blocks, width, palette);
      // Reflowing rows below a scroll-locked viewport would move the content
      // being read as estimates become exact. Visible rows still resolve on
      // demand; background work resumes when the user follows the tail again.
      if (scroll === 0) reflowBatch(layout, state);
      const window = reflowWindow(layout, Math.max(0, height), scroll, state);
      return {
        rows: visibleRows(layout, window, Math.max(0, height)),
        maxScroll: window.maxScroll,
        pending: scroll === 0 && layout.pending > 0,
      };
    },

    invalidate(block) {
      if (block === undefined) {
        layouts = [];
        return;
      }
      for (const layout of layouts) {
        const index = layout.positions.get(block);
        if (index === undefined) continue;
        const entry = layout.entries[index];
        if (entry?.rows !== undefined) {
          entry.rows = undefined;
          layout.pending++;
        }
        layout.next = Math.max(layout.next, index);
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
