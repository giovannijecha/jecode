// Bounded, read-only workspace discovery without borrowing a shell.

import * as fs from "node:fs/promises";
import * as path from "node:path";
import { BoundedFileError, readBoundedFile } from "../bounded-file.ts";
import type { StableFileExpectation } from "../bounded-file.ts";
import {
  readStableDirectory,
  StableDirectoryError,
} from "../stable-directory.ts";
import type { StableDirectoryEntry } from "../stable-directory.ts";
import type { Tool, ToolContext } from "./types.ts";
import { optionalBool, optionalInt, optionalString, requireString } from "./args.ts";
import { displayPath, resolveExistingInRoot } from "./paths.ts";
import { leadingText } from "./text-boundary.ts";

const DEFAULT_RESULTS = 100;
const MAX_RESULTS = 500;
const MAX_VISITED = 20_000;
const MAX_FILE_BYTES = 1_000_000;
const MAX_MATCH_LINE = 500;
const MAX_GLOB_CHARS = 512;
const PORTABLE_SEARCH_CONCURRENCY = 8;
const SKIP = new Set([".git", ".hg", ".svn", "node_modules"]);

export type SearchFile = Readonly<{
  path: string;
  bytes: number;
  expected?: StableFileExpectation;
}>;

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
    const pending: SearchFile[] = [];
    let skipped = 0;

    const flushPending = async (): Promise<boolean> => {
      if (pending.length === 0) return false;
      const searched = await portableSearch(
        pending.splice(0),
        scoped,
        needle,
        sensitive,
        limit - found.length,
      );
      found.push(...searched.matches);
      skipped += searched.skipped;
      return found.length >= limit;
    };

    const walked = await walk(start, scoped, async (lexical, expected) => {
      const relative = displayPath(scoped.root, lexical);
      if (!match(relative)) return false;
      const bytes = Number(expected.size);
      if (bytes > MAX_FILE_BYTES) {
        skipped++;
        return false;
      }
      pending.push({ path: lexical, bytes, expected });
      return pending.length >= PORTABLE_SEARCH_CONCURRENCY && await flushPending();
    });

    if (found.length < limit) await flushPending();

    const extra = skipped === 0 ? "" : ` · skipped ${skipped} binary/large/unreadable`;
    return {
      output: found.length === 0 ? "[no matches]" : found.join("\n"),
      summary: `${summary(found.length, limit, walked.capped, "match", "matches")}${extra}`,
    };
  },
};

export async function portableSearch(
  files: readonly SearchFile[],
  ctx: ToolContext,
  needle: string,
  sensitive: boolean,
  limit: number,
  read?: SearchReader,
): Promise<{ matches: string[]; skipped: number }> {
  const found: string[] = [];
  let skipped = 0;
  for (let start = 0; start < files.length; start += PORTABLE_SEARCH_CONCURRENCY) {
    checkAbort(ctx.signal);
    const remaining = limit - found.length;
    if (remaining <= 0) break;
    const batch = await Promise.all(
      files.slice(start, start + PORTABLE_SEARCH_CONCURRENCY)
        .map((file) => searchFile(file, ctx, needle, sensitive, remaining, read)),
    );
    for (const searched of batch) {
      if (searched === undefined) {
        skipped++;
        continue;
      }
      for (const match of searched) {
        found.push(match);
        if (found.length >= limit) return { matches: found, skipped };
      }
    }
  }
  return { matches: found, skipped };
}

type SearchReader = (file: string, signal: AbortSignal | undefined) => Promise<Buffer>;

async function searchFile(
  file: SearchFile,
  ctx: ToolContext,
  needle: string,
  sensitive: boolean,
  limit: number,
  read?: SearchReader,
): Promise<string[] | undefined> {
  let data: Buffer;
  try {
    data = read === undefined
      ? await readVerifiedSearchFile(file, ctx)
      : await read(file.path, ctx.signal);
  } catch (error) {
    checkAbort(ctx.signal);
    if (skippable(error)) return undefined;
    throw error;
  }
  checkAbort(ctx.signal);
  if (data.byteLength > MAX_FILE_BYTES || data.includes(0)) return undefined;

  const found: string[] = [];
  const text = data.toString("utf8");
  for (const [index, line] of text.replace(/\r\n?/g, "\n").split("\n").entries()) {
    checkAbort(ctx.signal);
    const haystack = sensitive ? line : line.toLocaleLowerCase();
    if (!haystack.includes(needle)) continue;
    found.push(`${displayPath(ctx.root, file.path)}:${index + 1}:${clip(line)}`);
    if (found.length >= limit) break;
  }
  return found;
}

async function readVerifiedSearchFile(file: SearchFile, ctx: ToolContext): Promise<Buffer> {
  if (file.expected === undefined) {
    throw new BoundedFileError("changed", "search file has no verified identity");
  }
  return readBoundedFile(file.path, MAX_FILE_BYTES, {
    label: "search file",
    signal: ctx.signal,
    expected: file.expected,
    validate: async () => {
      const confirmed = await resolveExistingInRoot(ctx.root, file.path);
      if (confirmed !== file.path) {
        throw new BoundedFileError("changed", "search file left the workspace");
      }
    },
  });
}

type WalkResult = { capped: boolean };

async function walk(
  start: string,
  ctx: ToolContext,
  visit: (file: string, expected: StableFileExpectation) => Promise<boolean>,
): Promise<WalkResult> {
  const pending: Array<{
    directory: string;
    entries: Array<StableDirectoryEntry | undefined>;
    next: number;
  }> = [];
  let seen = 0;
  let buffered = 0;
  let capped = false;

  const enter = async (lexical: string): Promise<boolean> => {
    checkAbort(ctx.signal);
    const directory = await resolveExistingInRoot(ctx.root, lexical);
    const room = MAX_VISITED - seen - buffered;
    if (room <= 0) {
      capped = true;
      return false;
    }
    try {
      const inspected = await readStableDirectory(ctx.root, directory, {
        maxEntries: room,
        signal: ctx.signal,
      });
      if (inspected.capped) capped = true;
      const entries = [...inspected.entries];
      buffered += entries.length;
      pending.push({ directory, entries, next: 0 });
    } catch (error) {
      if (skippable(error)) return true;
      throw error;
    }
    return true;
  };

  if (!await enter(start)) return { capped: true };
  while (pending.length > 0) {
    checkAbort(ctx.signal);
    const frame = pending[pending.length - 1];
    if (frame === undefined) break;
    const index = frame.next++;
    const entry = frame.entries[index];
    if (entry === undefined) {
      pending.pop();
      continue;
    }
    frame.entries[index] = undefined;
    buffered--;

    seen++;
    const target = path.join(frame.directory, entry.name);
    if (entry.kind === "directory") {
      if (!SKIP.has(entry.name)) await enter(target);
    } else if (await visit(target, entry.expected)) {
      return { capped };
    }
  }
  return { capped };
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
  return `${leadingText(line, MAX_MATCH_LINE - 1)}…`;
}

function checkAbort(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("aborted");
}

function skippable(error: unknown): boolean {
  return error instanceof BoundedFileError || error instanceof StableDirectoryError ||
    ["EACCES", "EPERM", "ENOENT"].includes((error as NodeJS.ErrnoException).code ?? "");
}
