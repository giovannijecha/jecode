// Filesystem tools: read, list, write, edit.

import { createReadStream } from "node:fs";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import type { Tool } from "./types.ts";
import { optionalBool, optionalInt, requireString } from "./args.ts";
import { displayPath, resolveExistingInRoot, resolveWritableInRoot } from "./paths.ts";
import { atomicWrite } from "../atomic.ts";

const MAX_READ_CHARS = 60_000;
const MAX_LIST_CHARS = 60_000;
const MAX_LIST_ENTRIES = 2_000;

export const readFile: Tool = {
  name: "read_file",
  description:
    "Read a UTF-8 text file inside the workspace. Optionally start at a line " +
    "(1-based) and cap how many lines come back. Large files are truncated.",
  dangerous: false,
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
    const root = await resolveExistingInRoot(ctx.root, ".");
    const target = await resolveExistingInRoot(root, requireString(args, "path"));
    const offset = optionalInt(args, "offset");
    const limit = optionalInt(args, "limit");

    const { text, truncated } = await readRange(target, offset, limit);

    if (truncated) {
      return {
        output: `${text}\n\n[truncated at ${MAX_READ_CHARS} characters — read a narrower range]`,
        summary: `truncated at ${MAX_READ_CHARS} characters`,
      };
    }
    if (text === "") return { output: "[file is empty]", summary: "empty" };
    return { output: text, summary: `${count(text, "line")}` };
  },
};

export const listDir: Tool = {
  name: "list_dir",
  description: "List the entries of a directory inside the workspace. Directories end with a slash.",
  dangerous: false,
  input: {
    type: "object",
    properties: {
      path: { type: "string", description: "Directory relative to the workspace root. Defaults to the root." },
    },
    required: [],
  },
  async run(args, ctx) {
    const root = await resolveExistingInRoot(ctx.root, ".");
    const target = await resolveExistingInRoot(
      root,
      args.path === undefined ? "." : requireString(args, "path"),
    );
    const entries: string[] = [];
    let chars = 0;
    let truncated = false;
    const directory = await fs.opendir(target);
    for await (const entry of directory) {
      const label = entry.isDirectory() ? `${entry.name}/` : entry.name;
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

    const listing = entries
      .sort((a, b) => a.localeCompare(b))
      .join("\n");

    if (truncated) {
      return {
        output: `${listing}\n\n[truncated after ${entries.length} entries]`,
        summary: `${entries.length}+ entries`,
      };
    }
    return { output: listing, summary: plural(entries.length, "entry", "entries") };
  },
};

export const writeFile: Tool = {
  name: "write_file",
  description:
    "Create a file, or replace its entire contents. Parent directories are " +
    "created as needed. To change part of an existing file, prefer edit_file.",
  dangerous: true,
  input: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path relative to the workspace root." },
      content: { type: "string", description: "The full new contents of the file." },
    },
    required: ["path", "content"],
  },
  async preview(args, ctx) {
    const root = await resolveExistingInRoot(ctx.root, ".");
    const target = await resolveWritableInRoot(root, requireString(args, "path"));
    // A write against a file that is already there is a replacement, and the
    // user is owed the difference rather than a wall of green.
    return { before: await current(target), after: requireString(args, "content") };
  },
  async run(args, ctx) {
    const root = await resolveExistingInRoot(ctx.root, ".");
    const target = await resolveWritableInRoot(root, requireString(args, "path"));
    const content = requireString(args, "content");
    await fs.mkdir(path.dirname(target), { recursive: true });
    await unchangedSinceApproval(target, ctx.preview?.before);
    await atomicWrite(target, content);
    return {
      output: `wrote ${displayPath(root, target)} (${content.length} characters)`,
      summary: count(content, "line"),
    };
  },
};

export const editFile: Tool = {
  name: "edit_file",
  description:
    "Replace an exact string in a file. The old text must appear exactly once " +
    "unless replace_all is true — include enough surrounding context to make it unique.",
  dangerous: true,
  input: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path relative to the workspace root." },
      old_text: { type: "string", description: "Exact text to replace, including indentation." },
      new_text: { type: "string", description: "Text to put in its place." },
      replace_all: { type: "boolean", description: "Replace every occurrence instead of requiring one." },
    },
    required: ["path", "old_text", "new_text"],
  },
  async preview(args, ctx) {
    const root = await resolveExistingInRoot(ctx.root, ".");
    const target = await resolveExistingInRoot(root, requireString(args, "path"));
    const before = await current(target);
    // An edit that will not apply gets no preview: the run is about to say so
    // properly, and a diff of a match that does not exist would be a lie.
    try {
      return { before, after: applied(before, args).after };
    } catch {
      return undefined;
    }
  },
  async run(args, ctx) {
    const root = await resolveExistingInRoot(ctx.root, ".");
    const target = await resolveExistingInRoot(root, requireString(args, "path"));
    const before = await fs.readFile(target, "utf8");
    if (ctx.preview !== undefined && before !== ctx.preview.before) {
      throw new Error("file changed after the preview — inspect it and retry the edit");
    }
    const { after, made } = applied(before, args);

    await atomicWrite(target, after);
    return {
      output: `edited ${displayPath(root, target)} (${made} replacement${made === 1 ? "" : "s"})`,
      summary: plural(made, "replacement", "replacements"),
    };
  },
};

async function readRange(
  target: string,
  offset: number | undefined,
  limit: number | undefined,
): Promise<{ text: string; truncated: boolean }> {
  const firstLine = Math.max(1, offset ?? 1);
  const lineCount = limit === undefined ? undefined : Math.max(0, limit);
  const endLine = lineCount === undefined ? Number.POSITIVE_INFINITY : firstLine + lineCount;
  if (lineCount === 0) return { text: "", truncated: false };

  const source = createReadStream(target);
  const decoder = new TextDecoder();
  let text = "";
  let line = 1;
  let stopped = false;
  let truncated = false;

  const selected = (at: number): boolean => at >= firstLine && at < endLine;
  const append = (fragment: string): void => {
    if (fragment === "") return;
    const room = MAX_READ_CHARS - text.length;
    if (fragment.length > room) {
      if (room > 0) text += fragment.slice(0, room);
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

  try {
    for await (const chunk of source as AsyncIterable<Uint8Array>) {
      consume(decoder.decode(chunk, { stream: true }));
      if (stopped) break;
    }
    if (!stopped) consume(decoder.decode());
  } finally {
    source.destroy();
  }

  return { text, truncated };
}

/**
 * The edit worked out against a given text — the one place its rules live.
 *
 * Shared by `run` and `preview` on purpose: a preview computed by a second
 * implementation of the same rules is a preview that can disagree with what
 * the call then does, which is worse than showing nothing at all.
 */
function applied(before: string, args: Record<string, unknown>): { after: string; made: number } {
  const oldText = requireString(args, "old_text");
  const newText = args.new_text === "" ? "" : requireString(args, "new_text");
  const replaceAll = optionalBool(args, "replace_all") ?? false;
  const occurrences = before.split(oldText).length - 1;

  if (occurrences === 0) throw new Error("old_text was not found in the file");
  if (occurrences > 1 && !replaceAll) {
    throw new Error(
      `old_text appears ${occurrences} times — add surrounding context, or pass replace_all`,
    );
  }

  return {
    after: replaceAll ? before.split(oldText).join(newText) : before.replace(oldText, newText),
    made: replaceAll ? occurrences : 1,
  };
}

/** What is on disk now, or nothing at all — a file that is not there yet. */
async function current(target: string): Promise<string> {
  try {
    return await fs.readFile(target, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return "";
    throw error;
  }
}

async function unchangedSinceApproval(target: string, approved: string | undefined): Promise<void> {
  if (approved === undefined) return;
  if ((await current(target)) !== approved) {
    throw new Error("file changed after the preview — inspect it and retry the write");
  }
}

function count(text: string, noun: string): string {
  return plural(text.split("\n").length, noun, `${noun}s`);
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}
