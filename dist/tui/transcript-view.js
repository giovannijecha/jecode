// Incremental transcript layout for long-running sessions.
import { render } from "./blocks.js";
/**
 * Keep block rows and cumulative row ends between frames.
 *
 * The app mutates semantic blocks while streaming, so it marks that one block
 * dirty. Appends then cost one render, stable frames cost no block renders,
 * and viewport selection starts with a binary search instead of flattening or
 * scanning the complete transcript.
 */
export function transcriptRenderer(draw = render) {
    let source;
    let layoutWidth = -1;
    let layoutPalette;
    let entries = [];
    let ends = [];
    let positions = new WeakMap();
    const dirty = new Set();
    return {
        viewport(blocks, width, height, scroll, palette) {
            sync(blocks, width, palette);
            const visibleHeight = Math.max(0, height);
            const totalRows = ends.at(-1) ?? 0;
            const maxScroll = Math.max(0, totalRows - visibleHeight);
            const offset = Math.min(Math.max(0, scroll), maxScroll);
            const start = Math.max(0, totalRows - visibleHeight - offset);
            const end = start + visibleHeight;
            const rows = [];
            let index = firstEndingAfter(start);
            let at = index === 0 ? 0 : (ends[index - 1] ?? 0);
            while (index < entries.length && at < end) {
                const blockRows = entries[index]?.rows ?? [];
                rows.push(...blockRows.slice(Math.max(0, start - at), Math.min(blockRows.length, end - at)));
                at = ends[index] ?? at;
                index++;
            }
            while (rows.length < visibleHeight)
                rows.unshift("");
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
            positions = new WeakMap();
            dirty.clear();
        },
    };
    function sync(blocks, width, palette) {
        if (source !== blocks ||
            layoutWidth !== width ||
            layoutPalette !== palette ||
            blocks.length < entries.length ||
            edgeChanged(blocks)) {
            rebuild(blocks, width, palette);
            return;
        }
        append(blocks, width, palette);
        refreshDirty(width, palette);
    }
    function rebuild(blocks, width, palette) {
        source = blocks;
        layoutWidth = width;
        layoutPalette = palette;
        entries = [];
        ends = [];
        positions = new WeakMap();
        dirty.clear();
        append(blocks, width, palette);
    }
    function append(blocks, width, palette) {
        let total = ends.at(-1) ?? 0;
        for (let index = entries.length; index < blocks.length; index++) {
            const block = blocks[index];
            if (block === undefined)
                continue;
            const rows = draw(block, width, palette);
            entries.push({ block, rows });
            positions.set(block, index);
            dirty.delete(block);
            total += rows.length;
            ends.push(total);
        }
    }
    function refreshDirty(width, palette) {
        if (dirty.size === 0)
            return;
        const changed = [...dirty]
            .map((block) => positions.get(block))
            .filter((index) => index !== undefined)
            .sort((left, right) => left - right);
        dirty.clear();
        if (changed.length === 0)
            return;
        const first = changed[0];
        for (const index of changed) {
            const entry = entries[index];
            if (entry !== undefined)
                entry.rows = draw(entry.block, width, palette);
        }
        recomputeEnds(first);
    }
    function recomputeEnds(from) {
        let total = from === 0 ? 0 : (ends[from - 1] ?? 0);
        for (let index = from; index < entries.length; index++) {
            total += entries[index]?.rows.length ?? 0;
            ends[index] = total;
        }
    }
    function edgeChanged(blocks) {
        if (entries.length === 0)
            return false;
        const lastShared = Math.min(blocks.length, entries.length) - 1;
        return lastShared < 0 ||
            entries[0]?.block !== blocks[0] ||
            entries[lastShared]?.block !== blocks[lastShared];
    }
    function firstEndingAfter(row) {
        let low = 0;
        let high = ends.length;
        while (low < high) {
            const middle = Math.floor((low + high) / 2);
            if ((ends[middle] ?? 0) <= row)
                low = middle + 1;
            else
                high = middle;
        }
        return low;
    }
}
