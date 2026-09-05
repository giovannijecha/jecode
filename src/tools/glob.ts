// Bounded workspace glob matching without regular-expression backtracking.

import * as path from "node:path";

const MAX_GLOB_CHARS = 512;

export function glob(pattern: string): (relative: string) => boolean {
  const normalized = pattern.replace(/\\/g, "/");
  if (normalized.length > MAX_GLOB_CHARS) {
    throw new Error(`"pattern" must be at most ${MAX_GLOB_CHARS} characters`);
  }
  const tokens = tokenizeGlob(normalized.toLowerCase());
  const basenameOnly = !normalized.includes("/");
  return (relative) => {
    const candidate = relative.replace(/\\/g, "/");
    const target = (basenameOnly ? path.posix.basename(candidate) : candidate).toLowerCase();
    return matchGlob(tokens, Array.from(target));
  };
}

type GlobToken =
  | { kind: "literal"; value: string }
  | { kind: "one" | "star" | "globstar" | "globdir-start" | "globdir-body" };

function tokenizeGlob(pattern: string): GlobToken[] {
  const chars = Array.from(pattern);
  const tokens: GlobToken[] = [];
  let index = 0;

  while (index < chars.length) {
    const char = chars[index] as string;
    if (char === "*") {
      let end = index + 1;
      while (chars[end] === "*") end++;
      if (end - index >= 2) {
        if (chars[end] === "/") {
          tokens.push({ kind: "globdir-start" }, { kind: "globdir-body" });
          index = end + 1;
        } else {
          tokens.push({ kind: "globstar" });
          index = end;
        }
      } else {
        tokens.push({ kind: "star" });
        index = end;
      }
      continue;
    }
    tokens.push(char === "?" ? { kind: "one" } : { kind: "literal", value: char });
    index++;
  }

  return tokens;
}

/** Thompson-style wildcard matching: O(pattern × path), with no regex backtracking. */
function matchGlob(tokens: readonly GlobToken[], text: readonly string[]): boolean {
  let states = epsilonClosure(new Set([0]), tokens);

  for (const char of text) {
    const next = new Set<number>();
    for (const state of states) {
      const token = tokens[state];
      if (token === undefined) continue;
      switch (token.kind) {
        case "literal":
          if (token.value === char) next.add(state + 1);
          break;
        case "one":
          if (char !== "/") next.add(state + 1);
          break;
        case "star":
          if (char !== "/") next.add(state);
          break;
        case "globstar":
          next.add(state);
          break;
        case "globdir-body":
          next.add(state);
          if (char === "/") next.add(state + 1);
          break;
        case "globdir-start":
          break;
      }
    }
    states = epsilonClosure(next, tokens);
    if (states.size === 0) return false;
  }

  return epsilonClosure(states, tokens).has(tokens.length);
}

function epsilonClosure(seed: Set<number>, tokens: readonly GlobToken[]): Set<number> {
  const states = new Set(seed);
  const pending = [...seed];

  while (pending.length > 0) {
    const state = pending.pop() as number;
    const token = tokens[state];
    const targets = token?.kind === "globdir-start"
      ? [state + 1, state + 2]
      : token?.kind === "star" || token?.kind === "globstar"
      ? [state + 1]
      : [];
    for (const target of targets) {
      if (states.has(target)) continue;
      states.add(target);
      pending.push(target);
    }
  }

  return states;
}
