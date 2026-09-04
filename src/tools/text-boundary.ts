// Shared size and truncation boundaries for text handled by workspace tools.

import { BoundedFileError, readBoundedText } from "../bounded-file.ts";
export { leadingText, trailingText } from "../text-boundary.ts";

export const MAX_EDITABLE_BYTES = 4_000_000;
export const MAX_EDITABLE_CHARS = 1_000_000;
export const MAX_EDITABLE_LINES = 20_000;

type ReadOptions = {
  label?: string;
};

/** Read a regular UTF-8 file without allowing an unbounded allocation. */
export async function readEditableText(
  file: string,
  options: ReadOptions = {},
): Promise<string> {
  const label = options.label ?? "file";
  try {
    const text = await readBoundedText(file, MAX_EDITABLE_BYTES, { label });
    assertEditableText(text, label);
    return text;
  } catch (error) {
    if (error instanceof BoundedFileError && error.kind === "too-large") {
      throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
    }
    throw error;
  }
}

/** Reject whole-file writes that exceed Jecode's mutation budget. */
export function assertEditableText(text: string, label = "content"): void {
  if (Buffer.byteLength(text, "utf8") > MAX_EDITABLE_BYTES) {
    throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
  }
  if (text.length > MAX_EDITABLE_CHARS) {
    throw limitError(label, `${MAX_EDITABLE_CHARS} characters`);
  }
  if (lineCount(text) > MAX_EDITABLE_LINES) {
    throw limitError(label, `${MAX_EDITABLE_LINES} lines`);
  }
}

/** Check a replacement's size before constructing the resulting string. */
export function assertReplacementFits(
  before: string,
  oldText: string,
  newText: string,
  replacements: number,
): void {
  const projectedChars =
    before.length + replacements * (newText.length - oldText.length);
  if (projectedChars > MAX_EDITABLE_CHARS) {
    throw limitError("edited content", `${MAX_EDITABLE_CHARS} characters`);
  }

  const projectedLines =
    lineCount(before) +
    replacements * (newlineCount(newText) - newlineCount(oldText));
  if (projectedLines > MAX_EDITABLE_LINES) {
    throw limitError("edited content", `${MAX_EDITABLE_LINES} lines`);
  }
}

function lineCount(text: string): number {
  return newlineCount(text) + 1;
}

function newlineCount(text: string): number {
  let lines = 0;
  for (let index = text.indexOf("\n"); index !== -1; index = text.indexOf("\n", index + 1)) {
    lines += 1;
  }
  return lines;
}

function limitError(label: string, limit: string): Error {
  return new Error(`${label} exceeds the whole-file mutation limit of ${limit}`);
}
