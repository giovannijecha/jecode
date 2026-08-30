// Project semantic blocks into a compact transcript for pipes.
import { render } from "./tui/blocks.js";
export function renderBatch(block, width, pal) {
    const rows = [];
    let lastWasBlank = false;
    for (const rendered of render(block, width, pal)) {
        const line = rendered.trimEnd();
        const blank = line === "";
        if (blank && lastWasBlank)
            continue;
        rows.push(line);
        lastWasBlank = blank;
    }
    while (rows.at(-1) === "")
        rows.pop();
    return rows;
}
