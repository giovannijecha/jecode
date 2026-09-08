// A turn-local Responses connection. Only an exact compatible prefix can use
// previous_response_id; history itself always contains complete provider output.
import { createHash } from "node:crypto";
import type { ResponseStage, SendRequest, TransportObservation } from "../types.ts";
import type { OpenAIResponse } from "./openai-wire.ts";
import { openAIResponseStage, openAIStreamProgress, openAITerminalEvent } from "./openai-stream.ts";
import { addBounded, sseStreamCharacterLimit } from "./stream-limits.ts";
import { SocketChannel } from "./websocket.ts";
import { providerWireError } from "./failure.ts";
import { TransportError } from "./transport-error.ts";
import { MODEL_PROGRESS_TIMEOUT_MS, modelProgressTimeout } from "./stream-timeout.ts";

export type ResponsesBody = Record<string, unknown> & { input: unknown[] };
type Baseline = { id: string; settings: string; input: string[] };

export class ResponsesSession {
  #channel: SocketChannel | undefined;
  #authorization: string | undefined;
  #baseline: Baseline | undefined;
  #httpOnly = false;
  #closed = false;
  #busy = false;
  #routingState: string | undefined;

  close(): void {
    this.#closed = true;
    this.#routingState = undefined;
    this.#reset();
  }

  httpHeaders(headers: Record<string, string>): Record<string, string> {
    return this.#routingState === undefined ? headers : { ...headers, "x-codex-turn-state": this.#routingState };
  }

  observeHttpHeaders(headers: Headers): void {
    const state = headers.get("x-codex-turn-state");
    if (this.#routingState === undefined && state !== null && /^[\x20-\x7e]{1,4096}$/u.test(state)) {
      this.#routingState = state;
    }
  }

  remember(body: ResponsesBody, response: OpenAIResponse): void {
    this.#baseline = typeof response.id === "string" && response.id.length <= 512 &&
        response.id !== "" && response.status === "completed" && Array.isArray(response.output)
      ? { id: response.id, settings: settingsHash(body),
        input: [...body.input, ...response.output].map(hash) }
      : undefined;
  }

  async *events(
    url: string,
    headers: Record<string, string>,
    body: ResponsesBody,
    request: SendRequest,
    http: () => Promise<AsyncIterable<unknown>>,
  ): AsyncGenerator<unknown> {
    if (this.#closed || this.#busy) throw new Error("Responses transport scope is not available");
    this.#busy = true;
    let completed = false;
    let observation: TransportObservation | undefined;
    let responseStage: ResponseStage = "awaiting";
    const started = performance.now();
    try {
      request.signal?.throwIfAborted();
      const authorization = hash([url, headers["authorization"], headers["chatgpt-account-id"], headers["session-id"]]);
      if (this.#authorization !== authorization) {
        this.#reset();
        this.#routingState = undefined;
        this.#authorization = authorization;
      }
      let reused = this.#channel?.ready === true;
      if (!reused && !this.#httpOnly) {
        this.#reset();
        request.onStatus?.("Connecting");
        try {
          this.#channel = new SocketChannel(url.replace(/^http/u, "ws"), headers);
          await this.#channel.connected(request.signal);
        } catch {
          this.#reset();
          request.signal?.throwIfAborted();
          // No response.create was sent. HTTP can safely try the full request,
          // including its normal explicit rejection and authentication handling.
          this.#httpOnly = true;
        }
      }
      if (this.#httpOnly) {
        yield* await http();
        return;
      }
      const channel = this.#channel!;
      let total = 0;
      const maximum = sseStreamCharacterLimit(request.maxTokens);
      const base = this.#baseline;
      let incremental = base !== undefined && settingsHash(body) === base.settings &&
        body.input.length > base.input.length && base.input.every((item, index) => item === hash(body.input[index]));
      const { stream: _stream, ...wire } = body;
      for (let attempt = 0; attempt < 2; attempt++) {
        request.signal?.throwIfAborted();
        responseStage = "awaiting";
        const text = JSON.stringify({ type: "response.create", ...wire,
          ...(incremental ? { previous_response_id: base!.id, input: body.input.slice(base!.input.length) } : {}) });
        channel.send(text);
        observation = { transport: "websocket", connectMs: Math.round(performance.now() - started),
          requestBytes: Buffer.byteLength(text), reused, incremental, fallback: attempt > 0, responseStage };
        request.onTransport?.(observation);
        request.onStatus?.("Waiting for model");
        let progressAt = performance.now();
        let observed = false;
        for (;;) {
          const remaining = MODEL_PROGRESS_TIMEOUT_MS - (performance.now() - progressAt);
          if (remaining <= 0) throw modelProgressTimeout();
          const text = await channel.next(remaining, request.signal, modelProgressTimeout);
          try { total = addBounded(total, text.length, maximum, "WebSocket stream"); }
          catch { throw new TransportError("stream-limit", `WebSocket stream exceeded ${maximum} characters`); }
          let event: unknown;
          try { event = JSON.parse(text); }
          catch { throw new TransportError("invalid-json", "WebSocket event contained invalid JSON"); }
          responseStage = openAIResponseStage(event, responseStage);
          if (incremental && attempt === 0 && !observed && predecessorMissing(event)) {
            this.#baseline = undefined;
            incremental = false;
            reused = true;
            break;
          }
          if (!observed) {
            const rejection = requestRejection(event);
            if (rejection !== undefined) throw rejection;
          }
          observed = true;
          if (openAIStreamProgress(event)) progressAt = performance.now();
          completed = openAITerminalEvent(event);
          yield event;
          if (completed) return;
        }
      }
    } finally {
      try {
        if (observation !== undefined && this.#channel !== undefined) {
          request.onTransport?.({ ...observation, ...this.#channel.statistics, responseStage });
        }
      } finally {
        this.#busy = false;
        if (!completed || request.signal?.aborted) this.#reset();
      }
    }
  }

  #reset(): void {
    this.#channel?.close();
    this.#channel = undefined;
    this.#baseline = undefined;
  }
}

function hash(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}

function settingsHash(body: ResponsesBody): string {
  const { input: _input, ...settings } = body;
  return hash(settings);
}

function predecessorMissing(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const event = value as { type?: unknown; error?: { code?: unknown }; code?: unknown };
  return event.type === "error" && (event.error?.code ?? event.code) === "previous_response_not_found";
}

// WebSocket request rejections carry HTTP-like status inside the first event.
// Preserve it for account refresh/context recovery only before stream progress;
// a later error must never authorize an automatic generation replay.
function requestRejection(value: unknown): Error | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const event = value as { type?: unknown; status?: unknown;
    error?: { code?: unknown; type?: unknown; message?: unknown } };
  if (event.type !== "error" || typeof event.status !== "number" ||
      !Number.isInteger(event.status) || event.status < 400 || event.status > 599) return undefined;
  const error = providerWireError("openai request rejected",
    typeof event.error?.message === "string" ? event.error.message : undefined,
    { code: event.error?.code, type: event.error?.type }) as Error & { code?: string };
  return Object.assign(error, { status: event.status, body: JSON.stringify({ error: { code: error.code } }) });
}
