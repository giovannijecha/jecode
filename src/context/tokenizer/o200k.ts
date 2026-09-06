// Owned o200k_base ordinary-text counter. Special-looking user text stays literal.
import { createHash } from "node:crypto";
import { countPiece } from "./bpe.ts";
import { vocabulary } from "./vocabulary.ts";

// OpenAI tiktoken's o200k_base split, with scoped ASCII case-insensitivity
// expanded for JavaScript. Vocabulary and expression attribution: assets/tokenizers/LICENSE.
const pattern = /[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]*[\p{Ll}\p{Lm}\p{Lo}\p{M}]+(?:'[sStTmMdD]|'[rR][eE]|'[vV][eE]|'[lL][lL])?|[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]+[\p{Ll}\p{Lm}\p{Lo}\p{M}]*(?:'[sStTmMdD]|'[rR][eE]|'[vV][eE]|'[lL][lL])?|\p{N}{1,3}| ?[^\p{White_Space}\p{L}\p{N}]+[\r\n/]*|\p{White_Space}*[\r\n]+|\p{White_Space}+(?!\P{White_Space})|\p{White_Space}+/gu;
const cache = new Map<string, number>();
const CHUNK = 8_192;

export async function countO200k(text: string, signal?: AbortSignal): Promise<number> {
  signal?.throwIfAborted();
  const key = createHash("sha256").update(text).digest("hex");
  const cached = cache.get(key);
  if (cached !== undefined) return cached;
  const ranks = await vocabulary();
  signal?.throwIfAborted();
  let count = 0;
  // Repetitive logs or minified data must not repeat the same merge work.
  // Keep text only within this call, bounded independently of input size.
  const chunks = new Map<string, number>();
  let chunkUnits = 0;
  for (let start = 0; start < text.length;) {
    let end = Math.min(start + CHUNK, text.length);
    if (end < text.length && /[\uD800-\uDBFF]/u.test(text[end - 1] as string)) end--;
    const chunk = text.slice(start, end);
    let tokens = chunks.get(chunk);
    if (tokens === undefined) {
      tokens = 0;
      for (const match of chunk.matchAll(pattern)) tokens += countPiece(Buffer.from(match[0], "utf8"), ranks);
      while (chunkUnits + chunk.length > 131_072) {
        const oldest = chunks.keys().next().value as string;
        chunks.delete(oldest);
        chunkUnits -= oldest.length;
      }
      chunks.set(chunk, tokens);
      chunkUnits += chunk.length;
    }
    count += tokens;
    // Bounded slices may split a token/pre-token. Reserve boundary room; this
    // is an input estimate, not a claim of exact whole-request tokenization.
    if (end < text.length) count += 8;
    start = end;
    await new Promise<void>((resolve) => setImmediate(resolve));
    signal?.throwIfAborted();
  }
  if (cache.size >= 2_048) cache.delete(cache.keys().next().value as string);
  cache.set(key, count);
  return count;
}
