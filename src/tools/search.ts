// Bounded, read-only workspace discovery without borrowing a shell.

import * as fs from "node:fs/promises";
import type { Dirent } from "node:fs";
import * as path from "node:path";
import type { Tool, ToolContext } from "./types.ts";
import { optionalBool, optionalInt, optionalString, requireString } from "./args.ts";
import { displayPath, resolveExistingInRoot } from "./paths.ts";
import { trySearchWithRipgrep } from "./ripgrep.ts";
import type { SearchFile } from "./ripgrep.ts";

const DEFAULT_RESULTS = 100;
const MAX_RESULTS = 500;
const MAX_VISITED = 20_000;
const MAX_FILE_BYTES = 1_000_000;
const MAX_MATCH_LINE = 500;
const MAX_GLOB_CHARS = 512;
const RG_PREFIX_BYTES = 2_000_000;
const RG_PREFIX_FILES = 500;
const MIN_RG_TAIL_BYTES = 2_000_000;
const MIN_RG_TAIL_FILES = 500;
const SKIP = new Set([".git", ".hg", ".svn", "node_modules"]);

export const findFiles: Tool = {
  name: "find_files",
  description:
    "Find files inside the workspace by glob (for example **/*.ts). Skips dependency and VCS " +
    "directories, never follows symlinks, and returns a bounded list.",
  dangerous: false,
  concurrency: "shared",
  input: {
    type: "object",
    properties: {
      pattern: {
        type: "string",
        description: "Glob matched against workspace-relative paths. Maximum 512 characters.",
      },
      path: { type: "string", description: "Directory to search, relative to the workspace root." },
      max_results: { type: "integer", description: "Maximum paths returned. Defaults to 100, caps at 500." },
    },
    required: ["pattern"],
  },
  async run(args, ctx) {
    const scoped = await canonicalContext(ctx);
    const start = await startAt(args, scoped);
    const match = glob(requireString(args, "pattern"));
    const limit = resultLimit(args);
    const found: string[] = [];
    const walked = await walk(start, scoped, async (file) => {
      const relative = displayPath(scoped.root, file);
      if (match(relative)) found.push(relative);
      return found.length >= limit;
    });
    found.sort((a, b) => a.localeCompare(b));
    return {
      output: found.length === 0 ? "[no matching files]" : found.join("\n"),
      summary: summary(found.length, limit, walked.capped, "file", "files"),
    };
  },
};

export const searchText: Tool = {
  name: "search_text",
  description:
    "Search UTF-8 text files inside the workspace for a literal string. Skips dependencies, VCS " +
    "directories, symlinks, binary files, and files over 1 MB; results are bounded.",
  dangerous: false,
  concurrency: "shared",
  input: {
    type: "object",
    properties: {
      query: { type: "string", description: "Literal text to find." },
      path: { type: "string", description: "Directory to search, relative to the workspace root." },
      pattern: {
        type: "string",
        description: "Optional file glob, for example **/*.ts. Maximum 512 characters.",
      },
      case_sensitive: { type: "boolean", description: "Defaults to false." },
      max_results: { type: "integer", description: "Maximum matching lines. Defaults to 100, caps at 500." },
    },
    required: ["query"],
  },
  async run(args, ctx) {
    const scoped = await canonicalContext(ctx);
    const start = await startAt(args, scoped);
    const query = requireString(args, "query");
    const sensitive = optionalBool(args, "case_sensitive") ?? false;
    const needle = sensitive ? query : query.toLocaleLowerCase();
    const pattern = optionalString(args, "pattern");
    const match = pattern === undefined || pattern === "" ? () => true : glob(pattern);
    const limit = resultLimit(args);
    const found: string[] = [];
    const tail: SearchFile[] = [];
    let skipped = 0;
    let prefixBytes = 0;
    let prefixFiles = 0;
    let tailBytes = 0;

    const walked = await walk(start, scoped, async (lexical) => {
      const relative = displayPath(scoped.root, lexical);
      if (!match(relative)) return false;

      const file = await resolveExistingInRoot(scoped.root, lexical);
      const info = await fs.stat(file);
      if (info.size > MAX_FILE_BYTES) {
        skipped++;
        return false;
      }

      const candidate = { path: file, bytes: info.size };
      if (
        prefixFiles + 1 > RG_PREFIX_FILES ||
        prefixBytes + info.size > RG_PREFIX_BYTES
      ) {
        tail.push(candidate);
        tailBytes += info.size;
        return false;
      }

      prefixFiles++;
      prefixBytes += info.size;
      const searched = await portableSearch(
        [candidate],
        scoped,
        needle,
        sensitive,
        limit - found.length,
      );
      found.push(...searched.matches);
      skipped += searched.skipped;
      return found.length >= limit;
    });

    const accelerated = preferRipgrep(tail, tailBytes)
      ? await trySearchWithRipgrep({
          files: tail,
          query,
          caseSensitive: sensitive,
          limit: limit - found.length,
          signal: ctx.signal,
        })
      : undefined;
    const portable = accelerated === undefined && tail.length > 0
      ? await portableSearch(tail, scoped, needle, sensitive, limit - found.length)
      : undefined;
    found.push(...(portable?.matches ?? accelerated?.matches.map((match) => (
      `${displayPath(scoped.root, match.path)}:${match.line}:${clip(match.text)}`
    )) ?? []));
    skipped += portable?.skipped ?? accelerated?.binaryPaths.length ?? 0;

    const extra = skipped === 0 ? "" : ` · skipped ${skipped} binary/large/unreadable`;
    return {
      output: found.length === 0 ? "[no matches]" : found.join("\n"),
      summary: `${summary(found.length, limit, walked.capped, "match", "matches")}${extra}`,
    };
  },
};

async function portableSearch(
  files: readonly SearchFile[],
  ctx: ToolContext,
  needle: string,
  sensitive: boolean,
  limit: number,
): Promise<{ matches: string[]; skipped: number }> {
  const found: string[] = [];
  let skipped = 0;
  for (const file of files) {
    checkAbort(ctx.signal);

    let text: string;
    try {
      const data = await fs.readFile(file.path);
      if (data.includes(0)) {
        skipped++;
        continue;
      }
      text = data.toString("utf8");
    } catch (error) {
      if (skippable(error)) {
        skipped++;
        continue;
      }
      throw error;
    }

    for (const [index, line] of text.replace(/\r\n?/g, "\n").split("\n").entries()) {
      checkAbort(ctx.signal);
      const haystack = sensitive ? line : line.toLocaleLowerCase();
      if (!haystack.includes(needle)) continue;
      found.push(`${displayPath(ctx.root, file.path)}:${index + 1}:${clip(line)}`);
      if (found.length >= limit) return { matches: found, skipped };
    }
  }
  return { matches: found, skipped };
}

function preferRipgrep(files: readonly SearchFile[], bytes: number): boolean {
  return files.length >= MIN_RG_TAIL_FILES || bytes >= MIN_RG_TAIL_BYTES;
}

type WalkResult = { capped: boolean };

async function walk(
  start: string,
  ctx: ToolContext,
  visit: (file: string) => Promise<boolean>,
): Promise<WalkResult> {
  const pending = [start];
  let seen = 0;

  while (pending.length > 0) {
    checkAbort(ctx.signal);
    const lexical = pending.pop() as string;
    const directory = await resolveExistingInRoot(ctx.root, lexical);
    let entries: Dirent<string>[];
    try {
      entries = await fs.readdir(directory, { withFileTypes: true, encoding: "utf8" });
    } catch (error) {
      if (skippable(error)) continue;
      throw error;
    }
    entries.sort((a, b) => a.name.localeCompare(b.name));

    for (const entry of entries) {
      checkAbort(ctx.signal);
      if (++seen > MAX_VISITED) return { capped: true };
      if (entry.isSymbolicLink()) continue;
      const target = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        if (!SKIP.has(entry.name)) pending.push(target);
      } else if (entry.isFile() && (await visit(target))) {
        return { capped: false };
      }
    }
  }
  return { capped: false };
}

async function startAt(args: Record<string, unknown>, ctx: ToolContext): Promise<string> {
  const candidate = optionalString(args, "path") ?? ".";
  const target = await resolveExistingInRoot(ctx.root, candidate);
  if (!(await fs.stat(target)).isDirectory()) throw new Error(`"path" is not a directory: ${candidate}`);
  return target;
}

async function canonicalContext(ctx: ToolContext): Promise<ToolContext> {
  return { ...ctx, root: await resolveExistingInRoot(ctx.root, ".") };
}

function resultLimit(args: Record<string, unknown>): number {
  const requested = optionalInt(args, "max_results") ?? DEFAULT_RESULTS;
  if (requested <= 0) throw new Error('"max_results" must be a positive integer');
  return Math.min(requested, MAX_RESULTS);
}

function glob(pattern: string): (relative: string) => boolean {
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

function summary(count: number, limit: number, capped: boolean, one: string, many: string): string {
  const noun = count === 1 ? one : many;
  if (count >= limit) return `${count} ${noun} · result limit`;
  if (capped) return `${count} ${noun} · scan limit`;
  return `${count} ${noun}`;
}

function clip(line: string): string {
  if (line.length <= MAX_MATCH_LINE) return line;
  return `${line.slice(0, MAX_MATCH_LINE - 1)}…`;
}

function checkAbort(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("aborted");
}

function skippable(error: unknown): boolean {
  return ["EACCES", "EPERM", "ENOENT"].includes((error as NodeJS.ErrnoException).code ?? "");
}
