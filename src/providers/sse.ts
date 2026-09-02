// Server-sent events, read off a fetch response body.
//
// The format is small: `field: value` lines, a blank line ends an event. Only
// `data` matters here — both providers put the event discriminator inside the
// JSON payload, so the `event:` line is redundant and skipped.

import {
  addBounded,
  MAX_SSE_EVENT_CHARS,
} from "./stream-limits.ts";

export async function* readSseJson(
  body: ReadableStream<Uint8Array>,
  maximumChars: number,
): AsyncGenerator<unknown> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  const parser = new SseEventParser();
  let finished = false;
  let total = 0;

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      const text = decoder.decode(value, { stream: true });
      total = addBounded(total, text.length, maximumChars, "SSE stream");
      for (const payload of parser.push(text)) yield payload;
    }

    // A stream that ends without a trailing blank line still owes us its last
    // event.
    const text = decoder.decode();
    total = addBounded(total, text.length, maximumChars, "SSE stream");
    for (const payload of parser.push(text)) yield payload;
    const payload = parser.finish();
    if (payload !== undefined) yield payload;
    finished = true;
  } finally {
    if (!finished) await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

// Keep fragments in bounded groups. A provider may split one SSE line into
// hundreds of thousands of tiny chunks; repeatedly flattening the growing line
// would make parsing quadratic even if boundary scanning itself were linear.
class TextParts {
  readonly #groups: string[] = [];
  #pieces: string[] = [];
  #lastCodeUnit = "";
  length = 0;

  append(text: string): void {
    if (text === "") return;
    this.#pieces.push(text);
    this.#lastCodeUnit = text.at(-1)!;
    this.length += text.length;
    if (this.#pieces.length >= 256) this.#flush();
  }

  take(): string {
    this.#flush();
    const text = this.#groups.length === 1 ? this.#groups[0]! : this.#groups.join("");
    this.#groups.length = 0;
    this.#lastCodeUnit = "";
    this.length = 0;
    return text;
  }

  endsWithCarriageReturn(): boolean {
    return this.#lastCodeUnit === "\r";
  }

  #flush(): void {
    if (this.#pieces.length === 0) return;
    this.#groups.push(this.#pieces.length === 1 ? this.#pieces[0]! : this.#pieces.join(""));
    this.#pieces = [];
  }
}

export class SseEventParser {
  readonly #line = new TextParts();
  #data: string[] = [];
  #eventLength = 0;
  #hasLine = false;
  #pendingTerminatorLength = 0;

  *push(text: string): Generator<unknown> {
    let start = 0;

    for (;;) {
      const newline = text.indexOf("\n", start);
      if (newline === -1) {
        this.#line.append(text.slice(start));
        this.#assertPendingLineSize();
        return;
      }

      this.#line.append(text.slice(start, newline));
      const rawLine = this.#line.take();
      const crlf = rawLine.endsWith("\r");
      const line = crlf ? rawLine.slice(0, -1) : rawLine;

      if (line === "") {
        const payload = this.#finishEvent();
        if (payload !== undefined) yield payload;
      } else {
        this.#appendLine(line, crlf ? 2 : 1);
      }

      start = newline + 1;
    }
  }

  finish(): unknown {
    const line = this.#line.take();
    if (line !== "") {
      this.#appendLine(line, 0);
    } else if (this.#hasLine) {
      // A single trailing line terminator is part of an unterminated event.
      assertEventSize(this.#eventLength + this.#pendingTerminatorLength);
    }
    return this.#finishEvent();
  }

  #appendLine(line: string, terminatorLength: number): void {
    const separatorLength = this.#hasLine ? this.#pendingTerminatorLength : 0;
    assertEventSize(this.#eventLength + separatorLength + line.length);
    this.#eventLength += separatorLength + line.length;
    this.#hasLine = true;
    this.#pendingTerminatorLength = terminatorLength;

    if (line.startsWith("data:")) {
      this.#data.push(line.slice("data:".length).trimStart());
    }
  }

  #assertPendingLineSize(): void {
    const separatorLength = this.#hasLine ? this.#pendingTerminatorLength : 0;
    const remaining = MAX_SSE_EVENT_CHARS - this.#eventLength - separatorLength;
    // One trailing CR may turn out to be part of a split CRLF terminator.
    const splitCrlf = this.#line.length === remaining + 1 && this.#line.endsWithCarriageReturn();
    if (this.#line.length > remaining && !splitCrlf) {
      assertEventSize(MAX_SSE_EVENT_CHARS + 1);
    }
  }

  #finishEvent(): unknown {
    const data = this.#data.join("\n");
    this.#data = [];
    this.#eventLength = 0;
    this.#hasLine = false;
    this.#pendingTerminatorLength = 0;
    return parseData(data);
  }
}

function assertEventSize(length: number): void {
  if (length > MAX_SSE_EVENT_CHARS) {
    throw new Error(`SSE event exceeded ${MAX_SSE_EVENT_CHARS} characters`);
  }
}

function parseData(data: string): unknown {
  if (data === "" || data === "[DONE]") return undefined;

  try {
    return JSON.parse(data) as unknown;
  } catch {
    throw new Error("SSE event contained invalid JSON");
  }
}
