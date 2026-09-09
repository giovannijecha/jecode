// Seeded, mixed source/config/prose input without private files or network data.
export const CORPUS_SEED = 0x5ec0de;

export function mixedCorpus(characters: number, seed = CORPUS_SEED): string {
  if (!Number.isSafeInteger(characters) || characters < 1 || characters > 5_000_000) {
    throw new Error("corpus size must be between 1 and 5000000 characters");
  }
  let state = seed >>> 0;
  const random = () => { state = (Math.imul(state, 1664525) + 1013904223) >>> 0; return state; };
  const parts: string[] = [];
  let size = 0;
  while (size < characters) {
    const id = random().toString(36);
    const value = random();
    const part = [
      `export function item_${id}(value) { if (value == null) throw new Error("missing ${value}"); return {id: "${id}", value}; }\n`,
      JSON.stringify({ id, enabled: Boolean(value % 2), limit: value % 999, path: `src/${id}/index.ts` }) + "\n",
      `## Decision ${id}\nPreserve ordered writes, cancellation, and recovery. UTF-8: café 日本 🙂. Sample ${value}.\n`,
      `2026-09-09T00:00:00Z test=${id} duration=${value % 1000}ms result=passed\n`,
    ][(value >>> 16) % 4]!;
    parts.push(part);
    size += part.length;
  }
  // Avoid ending inside a surrogate pair; the size is part of each workload report.
  return parts.join("").slice(0, characters).replace(/[\uD800-\uDBFF]$/u, " ");
}
