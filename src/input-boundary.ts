// One bounded ingress for text that can become a user prompt.

import { Buffer } from "node:buffer";
import { StringDecoder } from "node:string_decoder";
import { MAX_TEXT_CODE_UNITS } from "./text-boundary.ts";

export const MAX_PROMPT_CODE_UNITS = MAX_TEXT_CODE_UNITS;
export const PROMPT_LIMIT_MESSAGE =
  `Prompt cannot exceed ${MAX_PROMPT_CODE_UNITS.toLocaleString("en-US")} UTF-16 code units`;

export class PromptLimitError extends Error {
  constructor() {
    super(PROMPT_LIMIT_MESSAGE);
    this.name = "PromptLimitError";
  }
}

export function assertPromptLength(length: number): void {
  if (length > MAX_PROMPT_CODE_UNITS) throw new PromptLimitError();
}

export function assertPromptAppend(current: number, added: number): void {
  if (added > MAX_PROMPT_CODE_UNITS - current) throw new PromptLimitError();
}

/**
 * Split raw UTF-8 input without letting one unterminated line grow past the
 * prompt boundary. Newline, CRLF, and a final line without a newline match the
 * line semantics used by batch mode.
 */
export async function* boundedInputLines(
  source: AsyncIterable<string | Uint8Array>,
): AsyncGenerator<string> {
  const decoder = new StringDecoder("utf8");
  let line = "";
  let pendingCr = false;

  const append = (text: string, from: number, to: number): void => {
    const length = to - from;
    assertPromptAppend(line.length, length);
    if (length > 0) line += text.slice(from, to);
  };

  const consume = function* (text: string): Generator<string> {
    if (text === "") return;
    let from = 0;
    if (pendingCr) {
      if (text.startsWith("\n")) from = 1;
      pendingCr = false;
      yield line;
      line = "";
    }

    while (from < text.length) {
      const cr = text.indexOf("\r", from);
      const lf = text.indexOf("\n", from);
      const newline = cr === -1 ? lf : lf === -1 ? cr : Math.min(cr, lf);
      if (newline === -1) {
        append(text, from, text.length);
        return;
      }
      append(text, from, newline);
      if (text[newline] === "\r" && newline + 1 === text.length) {
        pendingCr = true;
        return;
      }
      yield line;
      line = "";
      from = text[newline] === "\r" && text[newline + 1] === "\n"
        ? newline + 2
        : newline + 1;
    }
  };

  for await (const chunk of source) {
    const text = typeof chunk === "string" ? chunk : decoder.write(Buffer.from(chunk));
    yield* consume(text);
  }

  const tail = decoder.end();
  if (tail !== "") yield* consume(tail);
  if (pendingCr) {
    yield line;
    line = "";
  }
  if (line !== "") yield line;
}
