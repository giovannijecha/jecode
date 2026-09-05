// Semantic tool evidence built from arguments, previews, and results.

import type { ToolCallBlock } from "../types.ts";
import type { ToolPreview } from "../tools/types.ts";
import { graphemes } from "../text-boundary.ts";
import { condense, diff } from "../ui/diff.ts";
import type { Detail } from "./blocks.ts";

/** Unchanged rows kept either side of a change. */
const CONTEXT = 2;

/** The argument worth showing: the thing the call acts on. */
export function toolTarget(input: Record<string, unknown>): string {
  const { path, command } = input as { path?: unknown; command?: unknown };
  if (typeof command === "string") return command;
  if (typeof path === "string") return path;
  const rest = JSON.stringify(input);
  return rest === "{}" ? "" : rest;
}

/**
 * What the call is about to do, drawn before it is allowed to do it.
 *
 * The tool is asked first, because only it knows what is already on disk: a
 * write against an existing file is a replacement, and showing it as a page of
 * additions hides exactly the part worth approving. The fallback diffs what is
 * in the arguments, which is all there is when a tool has nothing to say.
 */
export function previewDetails(call: ToolCallBlock, look?: ToolPreview): Detail[] | undefined {
  if (look !== undefined) return changes(look.before, look.after);

  const input = call.input as { content?: unknown; old_text?: unknown; new_text?: unknown };
  if (call.name === "write_file" && typeof input.content === "string") {
    return changes("", input.content);
  }
  if (call.name === "edit_file" && typeof input.old_text === "string") {
    return changes(input.old_text, typeof input.new_text === "string" ? input.new_text : "");
  }

  return undefined;
}

/** Two texts as the rows of their difference, unchanged runs summed up. */
function changes(before: string, after: string): Detail[] | undefined {
  let oldLine = 1;
  let newLine = 1;
  const rows: Detail[] = [];

  for (const changed of condense(diff(before, after), CONTEXT)) {
    if (changed.kind === "gap") {
      rows.push({ kind: "gap", text: `… ${changed.skipped} unchanged` });
      oldLine += changed.skipped;
      newLine += changed.skipped;
      continue;
    }
    if (changed.kind === "keep") {
      rows.push({ kind: "keep", text: tabs(changed.text), oldLine, newLine });
      oldLine++;
      newLine++;
      continue;
    }
    if (changed.kind === "del") {
      rows.push({ kind: "del", text: tabs(changed.text), oldLine });
      oldLine++;
      continue;
    }
    rows.push({ kind: "add", text: tabs(changed.text), newLine });
    newLine++;
  }

  emphasizePairs(rows);
  return rows.length === 0 ? undefined : rows;
}

function emphasizePairs(rows: Detail[]): void {
  for (let index = 0; index < rows.length - 1; index++) {
    const removed = rows[index];
    const added = rows[index + 1];
    if (removed?.kind !== "del" || added?.kind !== "add") continue;
    if (rows[index - 1]?.kind === "del" || rows[index + 2]?.kind === "add") continue;

    const removedClusters = graphemes(removed.text);
    const addedClusters = graphemes(added.text);
    let prefix = 0;
    let start = 0;
    while (
      prefix < removedClusters.length &&
      prefix < addedClusters.length &&
      removedClusters[prefix] === addedClusters[prefix]
    ) {
      start += (removedClusters[prefix] as string).length;
      prefix++;
    }
    let suffix = 0;
    let removedSuffix = 0;
    let addedSuffix = 0;
    while (
      suffix < removedClusters.length - prefix &&
      suffix < addedClusters.length - prefix &&
      removedClusters[removedClusters.length - 1 - suffix] ===
        addedClusters[addedClusters.length - 1 - suffix]
    ) {
      removedSuffix += (removedClusters[removedClusters.length - 1 - suffix] as string).length;
      addedSuffix += (addedClusters[addedClusters.length - 1 - suffix] as string).length;
      suffix++;
    }
    while (
      suffix > 0 &&
      (!wordBoundary(removed.text, removedSuffix) || !wordBoundary(added.text, addedSuffix))
    ) {
      removedSuffix -= (removedClusters[removedClusters.length - suffix] as string).length;
      addedSuffix -= (addedClusters[addedClusters.length - suffix] as string).length;
      suffix--;
    }

    const removedLength = removed.text.length - start - removedSuffix;
    const addedLength = added.text.length - start - addedSuffix;
    if (removedLength > 0) removed.emphasis = { start, length: removedLength };
    if (addedLength > 0) added.emphasis = { start, length: addedLength };
  }
}

function wordBoundary(text: string, suffix: number): boolean {
  const at = text.length - suffix;
  if (at <= 0 || at >= text.length) return true;
  return /\w/.test(text[at - 1] as string) !== /\w/.test(text[at] as string);
}

/** What the call left behind, for the calls whose output is worth a look. */
export function producedDetails(call: ToolCallBlock, output: string): Detail[] | undefined {
  if (call.name === "run_command") return outputDetails(output, "out");
  // A read or a listing is already summed up on the right of its own row, and
  // the model is about to say what was in it. Printing it twice helps nobody.
  return undefined;
}

export function outputDetails(text: string, kind: "out"): Detail[] | undefined {
  const rows = split(text).map((line) => ({ kind, text: tabs(line) }));
  return rows.length === 0 ? undefined : rows;
}

function split(text: string): string[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map(tabs);
}

// A tab is a width the terminal decides and the row measurement cannot see.
function tabs(line: string): string {
  return line.replace(/\t/g, "  ");
}
