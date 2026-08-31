// Incremental transcript layout for long-running sessions.

import type { Palette } from "../ui/theme.ts";
import type { Block } from "./blocks.ts";
import type { RenderContext } from "./blocks.ts";
import { render } from "./blocks.ts";

type DrawBlock = (block: Block, width: number, palette: Palette, context?: RenderContext) => string[];
type RenderState = Omit<RenderContext, "previous">;

type Entry = {
  block: Block;
  rows: string[];
};

export type TranscriptViewport = {
  rows: string[];
  maxScroll: number;
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
 * Keep block rows and cumulative row ends between frames.
 *
 * The app mutates semantic blocks while streaming, so it marks that one block
 * dirty. Appends then cost one render, stable frames cost no block renders,
 * and viewport selection starts with a binary search instead of flattening or
 * scanning the complete transcript.
 */
export function transcriptRenderer(draw: DrawBlock = render): TranscriptRenderer {
  let source: readonly Block[] | undefined;
  let layoutWidth = -1;
  let layoutPalette: Palette | undefined;
  let entries: Entry[] = [];
  let ends: number[] = [];
  let positions = new WeakMap<Block, number>();
  const dirty = new Set<Block>();

  return {
    viewport(blocks, width, height, scroll, palette, state = {}) {
      sync(blocks, width, palette, state);
      const visibleHeight = Math.max(0, height);
      const totalRows = ends.at(-1) ?? 0;
      const maxScroll = Math.max(0, totalRows - visibleHeight);
      const offset = Math.min(Math.max(0, scroll), maxScroll);
      const start = Math.max(0, totalRows - visibleHeight - offset);
      const end = start + visibleHeight;
      const rows: string[] = [];

      let index = firstEndingAfter(start);
      let at = index === 0 ? 0 : (ends[index - 1] ?? 0);
      while (index < entries.length && at < end) {
        const blockRows = entries[index]?.rows ?? [];
        rows.push(...blockRows.slice(Math.max(0, start - at), Math.min(blockRows.length, end - at)));
        at = ends[index] ?? at;
        index++;
      }

      while (rows.length < visibleHeight) rows.unshift("");
      return { rows, maxScroll };
    },

    invalidate(block) {
      if (block !== undefined) {
        dirty.add(block);
        return;
      }
      source = undefined;
      entries = [];
      ends = [];
      positions = new WeakMap<Block, number>();
      dirty.clear();
    },
  };

  function sync(blocks: readonly Block[], width: number, palette: Palette, state: RenderState): void {
    if (
      source !== blocks ||
      layoutWidth !== width ||
      layoutPalette !== palette ||
      blocks.length < entries.length ||
      edgeChanged(blocks)
    ) {
      rebuild(blocks, width, palette, state);
      return;
    }

    append(blocks, width, palette, state);
    refreshDirty(width, palette, state);
  }

  function rebuild(blocks: readonly Block[], width: number, palette: Palette, state: RenderState): void {
    source = blocks;
    layoutWidth = width;
    layoutPalette = palette;
    entries = [];
    ends = [];
    positions = new WeakMap<Block, number>();
    dirty.clear();
    append(blocks, width, palette, state);
  }

  function append(blocks: readonly Block[], width: number, palette: Palette, state: RenderState): void {
    let total = ends.at(-1) ?? 0;
    for (let index = entries.length; index < blocks.length; index++) {
      const block = blocks[index];
      if (block === undefined) continue;
      const rows = draw(block, width, palette, { ...state, previous: blocks[index - 1] });
      entries.push({ block, rows });
      positions.set(block, index);
      dirty.delete(block);
      total += rows.length;
      ends.push(total);
    }
  }

  function refreshDirty(width: number, palette: Palette, state: RenderState): void {
    if (dirty.size === 0) return;
    const changed = [...dirty]
      .map((block) => positions.get(block))
      .filter((index): index is number => index !== undefined)
      .sort((left, right) => left - right);
    dirty.clear();
    if (changed.length === 0) return;

    const first = changed[0] as number;
    for (const index of changed) {
      const entry = entries[index];
      if (entry !== undefined) {
        entry.rows = draw(entry.block, width, palette, {
          ...state,
          previous: entries[index - 1]?.block,
        });
      }
    }
    recomputeEnds(first);
  }

  function recomputeEnds(from: number): void {
    let total = from === 0 ? 0 : (ends[from - 1] ?? 0);
    for (let index = from; index < entries.length; index++) {
      total += entries[index]?.rows.length ?? 0;
      ends[index] = total;
    }
  }

  function edgeChanged(blocks: readonly Block[]): boolean {
    if (entries.length === 0) return false;
    const lastShared = Math.min(blocks.length, entries.length) - 1;
    return lastShared < 0 ||
      entries[0]?.block !== blocks[0] ||
      entries[lastShared]?.block !== blocks[lastShared];
  }

  function firstEndingAfter(row: number): number {
    let low = 0;
    let high = ends.length;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      if ((ends[middle] ?? 0) <= row) low = middle + 1;
      else high = middle;
    }
    return low;
  }
}
