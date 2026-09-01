// Comparing two texts line by line.
//
// Written here rather than shelled out to `git diff`, because a tool result has
// to be drawable on a machine with no git, in a workspace that is not a repo,
// for text that was never on disk — the new half of an edit exists only in the
// call the model just made.

export type Change = "keep" | "add" | "del";
export type Row = { kind: Change; text: string };
/** A run of unchanged rows that was left out, and how many it stood for. */
export type Gap = { kind: "gap"; skipped: number };

/**
 * Above this many cells the table stops being worth building.
 *
 * The diff is quadratic in the two line counts, and it is drawn between two
 * keystrokes. Past the ceiling the honest answer is the coarse one — all of
 * the old, then all of the new — rather than a frame the user waits for.
 */
const CEILING = 250_000;

export function diff(before: string, after: string): Row[] {
  const a = lines(before);
  const b = lines(after);

  if (a.length * b.length > CEILING) {
    return [...a.map(del), ...b.map(add)];
  }

  const table = common(a, b);
  const rows: Row[] = [];

  let i = 0;
  let j = 0;
  const width = b.length + 1;

  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      rows.push({ kind: "keep", text: a[i] as string });
      i++;
      j++;
      continue;
    }
    // Deletions first on a tie, so a replaced line reads old-then-new — the
    // order the eye expects, and the order every other diff prints.
    if ((table[(i + 1) * width + j] as number) >= (table[i * width + j + 1] as number)) {
      rows.push(del(a[i] as string));
      i++;
    } else {
      rows.push(add(b[j] as string));
      j++;
    }
  }

  while (i < a.length) rows.push(del(a[i++] as string));
  while (j < b.length) rows.push(add(b[j++] as string));

  return rows;
}

/**
 * Drop the unchanged stretches, keeping `context` rows either side.
 *
 * A diff nobody can see the changes in is a wall of text with a colour in it.
 */
export function condense(rows: readonly Row[], context: number): (Row | Gap)[] {
  const near = new Set<number>();
  rows.forEach((row, index) => {
    if (row.kind === "keep") return;
    for (let k = index - context; k <= index + context; k++) near.add(k);
  });

  const out: (Row | Gap)[] = [];
  let skipped = 0;

  const flush = (): void => {
    if (skipped > 0) out.push({ kind: "gap", skipped });
    skipped = 0;
  };

  rows.forEach((row, index) => {
    if (near.has(index)) {
      flush();
      out.push(row);
    } else {
      skipped++;
    }
  });

  flush();
  return out;
}

/** Lengths of the longest common subsequence of every pair of suffixes. */
function common(a: readonly string[], b: readonly string[]): Uint32Array {
  const width = b.length + 1;
  const table = new Uint32Array((a.length + 1) * width);

  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i * width + j] =
        a[i] === b[j]
          ? (table[(i + 1) * width + j + 1] as number) + 1
          : Math.max(table[(i + 1) * width + j] as number, table[i * width + j + 1] as number);
    }
  }

  return table;
}

// A trailing newline is a property of the file, not a line of it: counting it
// as one would report every append as touching two lines instead of one.
function lines(text: string): string[] {
  if (text === "") return [];
  const split = text.split("\n");
  if (split.length > 1 && split[split.length - 1] === "") split.pop();
  return split;
}

function add(text: string): Row {
  return { kind: "add", text };
}

function del(text: string): Row {
  return { kind: "del", text };
}
