// Atomic file mutations share preview validation and exact replacement rules.

import * as fs from "node:fs/promises";
import * as path from "node:path";
import { atomicWrite } from "../atomic.ts";
import type { Tool, ToolContext, ToolOutput, ToolPreview } from "./types.ts";
import { optionalBool, requireString } from "./args.ts";
import {
  assertDirectWritableInRoot,
  displayPath,
  resolveDirectWritableInRoot,
  resolveExistingInRoot,
} from "./paths.ts";
import {
  assertEditableText,
  assertReplacementFits,
  MAX_EDITABLE_CHARS,
  MAX_EDITABLE_LINES,
  readEditableText,
} from "./text-boundary.ts";
import { count, plural } from "./file-summary.ts";

export type FileMutationDependencies = {
  atomicWrite: typeof atomicWrite;
};

const DEFAULT_MUTATION_DEPENDENCIES: FileMutationDependencies = { atomicWrite };

export const writeFile: Tool = {
  name: "write_file",
  description:
    "Create a file, or replace its entire contents. Parent directories are " +
    "created as needed. Whole-file changes are limited to " +
    `${MAX_EDITABLE_CHARS} characters and ${MAX_EDITABLE_LINES} lines. ` +
    "To change part of an existing file, prefer edit_file.",
  dangerous: true,
  concurrency: "exclusive",
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
    const target = await resolveDirectWritableInRoot(root, requireString(args, "path"));
    const content = requireString(args, "content", true);
    assertEditableText(content);
    // A write against a file that is already there is a replacement, and the
    // user is owed the difference rather than a wall of green.
    const before = await current(target);
    return { before: before.text, after: content, beforeExists: before.exists };
  },
  async run(args, ctx) {
    return runWriteFile(args, ctx);
  },
};

export async function runWriteFile(
  args: Record<string, unknown>,
  ctx: ToolContext,
  dependencies: FileMutationDependencies = DEFAULT_MUTATION_DEPENDENCIES,
): Promise<ToolOutput> {
  throwIfAborted(ctx.signal);
  const root = await resolveExistingInRoot(ctx.root, ".");
  const target = await resolveDirectWritableInRoot(root, requireString(args, "path"));
  const content = requireString(args, "content", true);
  assertEditableText(content);
  await fs.mkdir(path.dirname(target), { recursive: true });
  throwIfAborted(ctx.signal);
  await assertDirectWritableInRoot(root, target);
  const before = await current(target);
  assertApproved(before, ctx.preview, "write");
  await dependencies.atomicWrite(target, content, {
    signal: ctx.signal,
    async validate(phase) {
      await assertDirectWritableInRoot(root, target);
      if (phase === "before-rename") {
        await assertUnchanged(target, before, "write", ctx.preview !== undefined);
      }
    },
  });
  return {
    output: `wrote ${displayPath(root, target)} (${content.length} characters)`,
    summary: count(content, "line"),
  };
}

export const editFile: Tool = {
  name: "edit_file",
  description:
    "Replace an exact string in a file. The old text must appear exactly once " +
    "unless replace_all is true — include enough surrounding context to make it unique. " +
    `Whole-file changes are limited to ${MAX_EDITABLE_CHARS} characters and ` +
    `${MAX_EDITABLE_LINES} lines.`,
  dangerous: true,
  concurrency: "exclusive",
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
    const target = await resolveDirectWritableInRoot(root, requireString(args, "path"), true);
    const before = await current(target, true);
    // An edit that will not apply gets no preview: the run is about to say so
    // properly, and a diff of a match that does not exist would be a lie.
    try {
      return {
        before: before.text,
        after: applied(before.text, args).after,
        beforeExists: true,
      };
    } catch {
      return undefined;
    }
  },
  async run(args, ctx) {
    return runEditFile(args, ctx);
  },
};

export async function runEditFile(
  args: Record<string, unknown>,
  ctx: ToolContext,
  dependencies: FileMutationDependencies = DEFAULT_MUTATION_DEPENDENCIES,
): Promise<ToolOutput> {
  throwIfAborted(ctx.signal);
  const root = await resolveExistingInRoot(ctx.root, ".");
  const target = await resolveDirectWritableInRoot(root, requireString(args, "path"), true);
  const before = await current(target, true);
  assertApproved(before, ctx.preview, "edit");
  const { after, made } = applied(before.text, args);

  await dependencies.atomicWrite(target, after, {
    signal: ctx.signal,
    async validate(phase) {
      await assertDirectWritableInRoot(root, target, true);
      if (phase === "before-rename") {
        await assertUnchanged(target, before, "edit", ctx.preview !== undefined);
      }
    },
  });
  return {
    output: `edited ${displayPath(root, target)} (${made} replacement${made === 1 ? "" : "s"})`,
    summary: plural(made, "replacement", "replacements"),
  };
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("interrupted");
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
  const occurrences = countOccurrences(before, oldText);

  if (occurrences === 0) throw new Error("old_text was not found in the file");
  if (occurrences > 1 && !replaceAll) {
    throw new Error(
      `old_text appears ${occurrences} times — add surrounding context, or pass replace_all`,
    );
  }

  const made = replaceAll ? occurrences : 1;
  assertReplacementFits(before, oldText, newText, made);
  const replacement = () => newText;
  const after = replaceAll
    ? before.replaceAll(oldText, replacement)
    : before.replace(oldText, replacement);
  assertEditableText(after, "edited content");
  return { after, made };
}

type CurrentFile = { exists: boolean; text: string };

/** What is on disk now, preserving the difference between absent and empty. */
async function current(target: string, mustExist = false): Promise<CurrentFile> {
  try {
    return { exists: true, text: await readEditableText(target) };
  } catch (error) {
    if (!mustExist && (error as NodeJS.ErrnoException).code === "ENOENT") {
      return { exists: false, text: "" };
    }
    throw error;
  }
}

function countOccurrences(haystack: string, needle: string): number {
  let occurrences = 0;
  for (
    let index = haystack.indexOf(needle);
    index !== -1;
    index = haystack.indexOf(needle, index + needle.length)
  ) {
    occurrences += 1;
  }
  return occurrences;
}

function assertApproved(current: CurrentFile, preview: ToolPreview | undefined, operation: string): void {
  if (preview === undefined) return;
  const existenceChanged =
    preview.beforeExists !== undefined && current.exists !== preview.beforeExists;
  if (existenceChanged || current.text !== preview.before) {
    throw changedFile(operation, true);
  }
}

async function assertUnchanged(
  target: string,
  expected: CurrentFile,
  operation: string,
  previewed: boolean,
): Promise<void> {
  const onDisk = await current(target);
  if (onDisk.exists !== expected.exists || onDisk.text !== expected.text) {
    throw changedFile(operation, previewed);
  }
}

function changedFile(operation: string, previewed: boolean): Error {
  const when = previewed ? "after the preview" : "while preparing the change";
  return new Error(`file changed ${when} — inspect it and retry the ${operation}`);
}
