// Comparing two texts line by line.
//
// Written here rather than shelled out to `git diff`, because a tool result has
// to be drawable on a machine with no git, in a workspace that is not a repo,
// for text that was never on disk — the new half of an edit exists only in the
// call the model just made.
/**
 * Above this many cells the table stops being worth building.
 *
 * The diff is quadratic in the two line counts, and it is drawn between two
 * keystrokes. Past the ceiling the honest answer is the coarse one — all of
 * the old, then all of the new — rather than a frame the user waits for.
 */
const CEILING = 250_000;
export function diff(before, after) {
    const a = lines(before);
    const b = lines(after);
    if (a.length * b.length > CEILING) {
        return [...a.map(del), ...b.map(add)];
    }
    const table = common(a, b);
    const rows = [];
    let i = 0;
    let j = 0;
    const width = b.length + 1;
    while (i < a.length && j < b.length) {
        if (a[i] === b[j]) {
            rows.push({ kind: "keep", text: a[i] });
            i++;
            j++;
            continue;
        }
        // Deletions first on a tie, so a replaced line reads old-then-new — the
        // order the eye expects, and the order every other diff prints.
        if (table[(i + 1) * width + j] >= table[i * width + j + 1]) {
            rows.push(del(a[i]));
            i++;
        }
        else {
            rows.push(add(b[j]));
            j++;
        }
    }
    while (i < a.length)
        rows.push(del(a[i++]));
    while (j < b.length)
        rows.push(add(b[j++]));
    return rows;
}
/**
 * Drop the unchanged stretches, keeping `context` rows either side.
 *
 * A diff nobody can see the changes in is a wall of text with a colour in it.
 */
export function condense(rows, context) {
    const near = new Set();
    rows.forEach((row, index) => {
        if (row.kind === "keep")
            return;
        for (let k = index - context; k <= index + context; k++)
            near.add(k);
    });
    const out = [];
    let skipped = 0;
    const flush = () => {
        if (skipped > 0)
            out.push({ kind: "gap", skipped });
        skipped = 0;
    };
    rows.forEach((row, index) => {
        if (near.has(index)) {
            flush();
            out.push(row);
        }
        else {
            skipped++;
        }
    });
    flush();
    return out;
}
/** Lengths of the longest common subsequence of every pair of suffixes. */
function common(a, b) {
    const width = b.length + 1;
    const table = new Uint32Array((a.length + 1) * width);
    for (let i = a.length - 1; i >= 0; i--) {
        for (let j = b.length - 1; j >= 0; j--) {
            table[i * width + j] =
                a[i] === b[j]
                    ? table[(i + 1) * width + j + 1] + 1
                    : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
        }
    }
    return table;
}
// A trailing newline is a property of the file, not a line of it: counting it
// as one would report every append as touching two lines instead of one.
function lines(text) {
    const split = text.split("\n");
    if (split.length > 1 && split[split.length - 1] === "")
        split.pop();
    return split;
}
function add(text) {
    return { kind: "add", text };
}
function del(text) {
    return { kind: "del", text };
}
