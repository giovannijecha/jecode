// Bounded workspace file reads and stable directory listings.

import * as path from "node:path";
import { withStableFile } from "../bounded-file.ts";
import { readStableDirectory } from "../stable-directory.ts";
import type { Tool, ToolContext, ToolOutput } from "./types.ts";
import { optionalInt, optionalString, requireString } from "./args.ts";
import { resolveExistingInRoot } from "./paths.ts";
import { leadingText } from "./text-boundary.ts";
import { count, plural } from "./file-summary.ts";

const MAX_READ_CHARS = 60_000;
const MAX_LIST_CHARS = 60_000;
const MAX_LIST_ENTRIES = 2_000;
const READ_CHUNK_BYTES = 64 * 1024;
const MAX_READ_SCAN_BYTES = 16 * 1024 * 1024;

export type FileReadDependencies = {
  beforeOpen?(): void | Promise<void>;
};

export type DirectoryReadDependencies = {
  beforeOpen?(): void | Promise<void>;
};

export const readFile: Tool = {
  name: "read_file",
  description:
    "Read a regular UTF-8 text file inside the workspace. Optionally start at a line " +
    "(1-based) and cap how many lines come back. Large files are truncated.",
  dangerous: false,
  concurrency: "shared",
  input: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path relative to the workspace root." },
      offset: { type: "integer", description: "First line to return, 1-based." },
      limit: { type: "integer", description: "How many lines to return." },
    },
    required: ["path"],
  },
  async run(args, ctx) {
    return runReadFile(args, ctx);
  },
};

export async function runReadFile(
  args: Record<string, unknown>,
  ctx: ToolContext,
  dependencies: FileReadDependencies = {},
): Promise<ToolOutput> {
  const root = await resolveExistingInRoot(ctx.root, ".");
  const target = await resolveExistingInRoot(root, requireString(args, "path"));
  const offset = optionalInt(args, "offset");
  const limit = optionalInt(args, "limit");
  const { text, truncated, scanCapped } = await readRange(
    root,
    target,
    offset,
    limit,
    ctx.signal,
    dependencies,
  );

  if (truncated) {
    return {
      output: `${text}\n\n[truncated at ${MAX_READ_CHARS} characters — read a narrower range]`,
      summary: `truncated at ${MAX_READ_CHARS} characters`,
    };
  }
  if (scanCapped) {
    const notice =
      `[read stopped after scanning ${MAX_READ_SCAN_BYTES} bytes — use a smaller offset or search_text]`;
    return {
      output: text === "" ? notice : `${text}\n\n${notice}`,
      summary: `scan capped at ${MAX_READ_SCAN_BYTES} bytes`,
    };
  }
  if (text === "") return { output: "[file is empty]", summary: "empty" };
  return { output: text, summary: `${count(text, "line")}` };
}

export const listDir: Tool = {
  name: "list_dir",
  description: "List the entries of a directory inside the workspace. Directories end with a slash.",
  dangerous: false,
  concurrency: "shared",
  input: {
    type: "object",
    properties: {
      path: { type: "string", description: "Directory relative to the workspace root. Defaults to the root." },
    },
    required: [],
  },
  async run(args, ctx) {
    return runListDir(args, ctx);
  },
};

export async function runListDir(
  args: Record<string, unknown>,
  ctx: ToolContext,
  dependencies: DirectoryReadDependencies = {},
): Promise<ToolOutput> {
  const root = await resolveExistingInRoot(ctx.root, ".");
  const requested = optionalString(args, "path");
  const relative = requested === undefined || requested.trim() === "" ? "." : requested;
  const target = await resolveExistingInRoot(root, relative);
  const inspected = await readStableDirectory(root, target, {
    maxEntries: MAX_LIST_ENTRIES + 1,
    signal: ctx.signal,
    beforeOpen: dependencies.beforeOpen,
  });
  const entries: string[] = [];
  let chars = 0;
  let truncated = inspected.capped;
  for (const entry of inspected.entries) {
    const label = entry.kind === "directory" ? `${entry.name}/` : entry.name;
    const separator = entries.length === 0 ? 0 : 1;
    if (
      entries.length >= MAX_LIST_ENTRIES ||
      chars + separator + label.length > MAX_LIST_CHARS
    ) {
      truncated = true;
      break;
    }
    entries.push(label);
    chars += separator + label.length;
  }
  if (entries.length === 0) return { output: "[empty directory]", summary: "empty" };
  const listing = entries.join("\n");
  if (truncated) {
    return {
      output: `${listing}\n\n[truncated after ${entries.length} entries]`,
      summary: `${entries.length}+ entries`,
    };
  }
  return { output: listing, summary: plural(entries.length, "entry", "entries") };
}

async function readRange(
  root: string,
  target: string,
  offset: number | undefined,
  limit: number | undefined,
  signal: AbortSignal | undefined,
  dependencies: FileReadDependencies,
): Promise<{ text: string; truncated: boolean; scanCapped: boolean }> {
  throwIfAborted(signal);
  const firstLine = Math.max(1, offset ?? 1);
  const lineCount = limit === undefined ? undefined : Math.max(0, limit);
  const endLine = lineCount === undefined ? Number.POSITIVE_INFINITY : firstLine + lineCount;
  if (lineCount === 0) return { text: "", truncated: false, scanCapped: false };

  const result = await withStableFile(target, {
    label: "read file",
    signal,
    beforeOpen: dependencies.beforeOpen,
  }, async (handle, opened) => {
    const decoder = new TextDecoder();
    let text = "";
    let line = 1;
    let stopped = false;
    let truncated = false;
    let reachedEnd = false;

    const selected = (at: number): boolean => at >= firstLine && at < endLine;
    const append = (fragment: string): void => {
      if (fragment === "") return;
      const room = MAX_READ_CHARS - text.length;
      if (fragment.length > room) {
        text += leadingText(fragment, room);
        truncated = true;
        stopped = true;
        return;
      }
      text += fragment;
    };
    const consume = (chunk: string): void => {
      let start = 0;
      while (!stopped && start < chunk.length) {
        const newline = chunk.indexOf("\n", start);
        const end = newline === -1 ? chunk.length : newline;
        if (selected(line)) append(chunk.slice(start, end));
        if (stopped || newline === -1) return;

        const nextLine = line + 1;
        if (selected(line) && selected(nextLine)) append("\n");
        line = nextLine;
        if (line >= endLine) {
          stopped = true;
          return;
        }
        start = newline + 1;
      }
    };

    let position = 0;
    const buffer = Buffer.allocUnsafe(READ_CHUNK_BYTES);
    while (!stopped && position < MAX_READ_SCAN_BYTES) {
      throwIfAborted(signal);
      const length = Math.min(buffer.length, MAX_READ_SCAN_BYTES - position);
      const { bytesRead } = await handle.read(buffer, 0, length, position);
      throwIfAborted(signal);
      if (bytesRead === 0) {
        reachedEnd = true;
        break;
      }
      position += bytesRead;
      consume(decoder.decode(buffer.subarray(0, bytesRead), { stream: true }));
    }
    if (!stopped && BigInt(position) >= opened.size) reachedEnd = true;
    if (!stopped && reachedEnd) consume(decoder.decode());
    return {
      text,
      truncated,
      scanCapped: !stopped && !reachedEnd && position >= MAX_READ_SCAN_BYTES,
    };
  });
  const confirmed = await resolveExistingInRoot(root, target);
  if (path.relative(target, confirmed) !== "") {
    throw new Error("read file changed while it was being read");
  }
  return result;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("interrupted");
}
