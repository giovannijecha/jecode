// Optional native acceleration for bounded literal search.

import { spawn } from "node:child_process";
import * as path from "node:path";
import { shellEnvironment } from "../credential-safety.ts";
import { resolveExecutable } from "../executable.ts";

const MAX_BATCH_FILES = 256;
const MAX_BATCH_BYTES = 16_000_000;
const MAX_COMMAND_CHARS = 12_000;
const MAX_QUERY_CHARS = 4_000;
const MAX_EVENT_CHARS = 8_000_000;
const MAX_STDERR_CHARS = 16_000;

export type SearchFile = { path: string; bytes: number };
export type RipgrepMatch = { path: string; line: number; text: string };
export type RipgrepSearch = { matches: RipgrepMatch[]; binaryPaths: string[] };

type RipgrepOptions = {
  root: string;
  files: readonly SearchFile[];
  query: string;
  caseSensitive: boolean;
  limit: number;
  signal?: AbortSignal;
};

/** Return undefined when ripgrep is unavailable or cannot preserve the contract. */
export async function trySearchWithRipgrep(
  options: RipgrepOptions,
): Promise<RipgrepSearch | undefined> {
  if (options.files.length === 0) return { matches: [], binaryPaths: [] };
  if (options.query.length > MAX_QUERY_CHARS || options.query.includes("\0")) return undefined;
  const executable = resolveExecutable("rg", { rejectUnder: options.root });
  if (executable === undefined) return undefined;

  const matches: RipgrepMatch[] = [];
  const binaryPaths = new Set<string>();

  try {
    for (const batch of batches(options.files, options.query.length)) {
      throwIfAborted(options.signal);
      const searched = await searchBatch(executable, batch, options);
      if (searched === undefined) return undefined;
      for (const match of searched.matches) {
        if (matches.length >= options.limit) break;
        matches.push(match);
      }
      for (const file of searched.binaryPaths) binaryPaths.add(file);
      if (matches.length >= options.limit) break;
    }
  } catch (error) {
    if (options.signal?.aborted === true) throw abortReason(options.signal);
    return undefined;
  }

  return { matches, binaryPaths: [...binaryPaths] };
}

function batches(files: readonly SearchFile[], queryChars: number): SearchFile[][] {
  const groups: SearchFile[][] = [];
  let group: SearchFile[] = [];
  let bytes = 0;
  let chars = queryChars;

  for (const file of files) {
    const nextChars = chars + file.path.length + 1;
    if (
      group.length > 0 &&
      (group.length >= MAX_BATCH_FILES ||
        bytes + file.bytes > MAX_BATCH_BYTES ||
        nextChars > MAX_COMMAND_CHARS)
    ) {
      groups.push(group);
      group = [];
      bytes = 0;
      chars = queryChars;
    }
    group.push(file);
    bytes += file.bytes;
    chars += file.path.length + 1;
  }
  if (group.length > 0) groups.push(group);
  return groups;
}

function searchBatch(
  executable: string,
  files: readonly SearchFile[],
  options: RipgrepOptions,
): Promise<RipgrepSearch | undefined> {
  throwIfAborted(options.signal);
  const args = [
    "--json",
    "--no-config",
    "--fixed-strings",
    "--max-filesize",
    "1000000",
    "--max-count",
    String(options.limit),
    ...(options.caseSensitive ? [] : ["--ignore-case"]),
    "--",
    options.query,
    ...files.map((file) => file.path),
  ];

  return new Promise((resolve, reject) => {
    let child;
    try {
      child = spawn(executable, args, {
        cwd: path.dirname(executable),
        env: shellEnvironment(),
        windowsHide: true,
        stdio: ["ignore", "pipe", "pipe"],
      });
    } catch {
      resolve(undefined);
      return;
    }

    const matches: RipgrepMatch[] = [];
    const binaryPaths = new Set<string>();
    let buffered = "";
    let stderr = "";
    let invalid = false;
    let settled = false;

    const onAbort = () => child.kill();
    options.signal?.addEventListener("abort", onAbort, { once: true });
    if (options.signal?.aborted === true) onAbort();

    const finish = (value: RipgrepSearch | undefined, error?: Error): void => {
      if (settled) return;
      settled = true;
      options.signal?.removeEventListener("abort", onAbort);
      if (error !== undefined) reject(error);
      else resolve(value);
    };

    const consume = (line: string): void => {
      if (line === "" || invalid) return;
      try {
        const event = JSON.parse(line) as unknown;
        const parsed = ripgrepEvent(event);
        if (parsed?.kind === "match") matches.push(parsed.match);
        if (parsed?.kind === "binary") binaryPaths.add(parsed.path);
      } catch {
        invalid = true;
        child.kill();
      }
    };

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      buffered += chunk;
      if (buffered.length > MAX_EVENT_CHARS) {
        invalid = true;
        child.kill();
        return;
      }
      let newline = buffered.indexOf("\n");
      while (newline !== -1) {
        consume(buffered.slice(0, newline));
        buffered = buffered.slice(newline + 1);
        newline = buffered.indexOf("\n");
      }
    });
    child.stderr.on("data", (chunk: string) => {
      stderr = `${stderr}${chunk}`.slice(-MAX_STDERR_CHARS);
    });

    child.on("error", (error: NodeJS.ErrnoException) => {
      if (options.signal?.aborted === true) finish(undefined, abortReason(options.signal));
      else if (error.code === "ENOENT") finish(undefined);
      else finish(undefined);
    });
    child.on("close", (code) => {
      if (options.signal?.aborted === true) {
        finish(undefined, abortReason(options.signal));
        return;
      }
      consume(buffered);
      if (invalid || (code !== 0 && code !== 1) || stderr.trim() !== "") {
        finish(undefined);
        return;
      }
      const order = new Map(files.map((file, index) => [file.path, index]));
      finish({
        matches: matches
          .filter((match) => !binaryPaths.has(match.path))
          .sort((a, b) => (
            (order.get(a.path) ?? Number.MAX_SAFE_INTEGER) -
              (order.get(b.path) ?? Number.MAX_SAFE_INTEGER) ||
            a.line - b.line
          )),
        binaryPaths: [...binaryPaths],
      });
    });
  });
}

type ParsedEvent =
  | { kind: "match"; match: RipgrepMatch }
  | { kind: "binary"; path: string };

function ripgrepEvent(value: unknown): ParsedEvent | undefined {
  if (!record(value) || !record(value["data"])) return undefined;
  const data = value["data"];
  const file = textField(data["path"]);
  if (file === undefined) return undefined;

  if (value["type"] === "match") {
    const line = data["line_number"];
    const text = textField(data["lines"]);
    if (typeof line !== "number" || !Number.isInteger(line) || text === undefined) return undefined;
    return {
      kind: "match",
      match: { path: file, line, text: text.replace(/\r?\n$/, "") },
    };
  }
  if (value["type"] === "end" && typeof data["binary_offset"] === "number") {
    return { kind: "binary", path: file };
  }
  return undefined;
}

function textField(value: unknown): string | undefined {
  return record(value) && typeof value["text"] === "string" ? value["text"] : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted === true) throw abortReason(signal);
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error("interrupted");
}
