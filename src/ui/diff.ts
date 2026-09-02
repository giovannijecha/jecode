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
 * keystrokes. Common edges are removed before this limit is applied, so a
 * local edit in a large file stays local. If the changed middle itself crosses
 * the ceiling, the honest answer is the coarse one rather than a frame the
 * user waits for.
 */
const CEILING = 250_000;

export function diff(before: string, after: string): Row[] {
  const a = lines(before);
  const b = lines(after);
  const prefix = commonPrefix(a, b);
  const suffix = commonSuffix(a, b, prefix);
  const aEnd = a.length - suffix;
  const bEnd = b.length - suffix;
  const middleA = a.slice(prefix, aEnd);
  const middleB = b.slice(prefix, bEnd);
  const rows = a.slice(0, prefix).map(keep);

  if (middleA.length * middleB.length > CEILING) {
    for (const line of middleA) rows.push(del(line));
    for (const line of middleB) rows.push(add(line));
    appendSuffix(rows, a, aEnd);
    return rows;
  }

  const table = common(middleA, middleB);

  let i = 0;
  let j = 0;
  const width = middleB.length + 1;

  while (i < middleA.length && j < middleB.length) {
    if (middleA[i] === middleB[j]) {
      rows.push(keep(middleA[i] as string));
      i++;
      j++;
      continue;
    }
    // Deletions first on a tie, so a replaced line reads old-then-new — the
    // order the eye expects, and the order every other diff prints.
    if ((table[(i + 1) * width + j] as number) >= (table[i * width + j + 1] as number)) {
      rows.push(del(middleA[i] as string));
      i++;
    } else {
      rows.push(add(middleB[j] as string));
      j++;
    }
  }

  while (i < middleA.length) rows.push(del(middleA[i++] as string));
  while (j < middleB.length) rows.push(add(middleB[j++] as string));
  appendSuffix(rows, a, aEnd);

  return rows;
}

function commonPrefix(a: readonly string[], b: readonly string[]): number {
  const limit = Math.min(a.length, b.length);
  let length = 0;
  while (length < limit && a[length] === b[length]) length++;
  return length;
}

function commonSuffix(a: readonly string[], b: readonly string[], prefix: number): number {
  const limit = Math.min(a.length, b.length) - prefix;
  let length = 0;
  while (length < limit && a[a.length - length - 1] === b[b.length - length - 1]) length++;
  return length;
}

function appendSuffix(rows: Row[], source: readonly string[], start: number): void {
  for (let index = start; index < source.length; index++) rows.push(keep(source[index] as string));
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

function keep(text: string): Row {
  return { kind: "keep", text };
}
