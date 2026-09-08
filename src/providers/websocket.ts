// Authenticated native Node WebSockets (WebSocketInit is supported on Node
// 22.18+). Wire messages, queued events, and wait times remain bounded here.
import { MAX_SSE_EVENT_CHARS } from "./stream-limits.ts";
import { WebSocketUpgrade } from "./websocket-upgrade.ts";
import { TransportError } from "./transport-error.ts";
import type { TransportObservation } from "../types.ts";

// Node accepts WebSocketInit, while the global TypeScript declaration exposes
// only the browser overload. Keep this supported Node extension at one boundary.
const NodeWebSocket = WebSocket as unknown as {
  new(url: string, options: { headers: Record<string, string>; dispatcher: WebSocketUpgrade }): WebSocket;
};

export class SocketChannel {
  readonly #socket: WebSocket;
  readonly #upgrade = new WebSocketUpgrade();
  readonly #queue: string[] = [];
  #queuedChars = 0;
  #failure: Error | undefined;
  #wake: (() => void) | undefined;
  #receivedEvents = 0;
  #receivedChars = 0;
  #largestEventChars = 0;
  #opened = false;
  #openedAt: number | undefined;
  #lastMessageAt: number | undefined;
  #failedAt: number | undefined;
  #nativeError = false;

  constructor(url: string, headers: Record<string, string>) {
    this.#socket = new NodeWebSocket(url, { headers, dispatcher: this.#upgrade });
    this.#socket.addEventListener("open", () => {
      this.#opened = true; this.#openedAt = performance.now(); this.#wake?.();
    });
    this.#socket.addEventListener("error", (event) => {
      // close() can emit another native error after a local limit or timeout.
      // Only count errors observed before we begin our own teardown.
      if (this.#failure === undefined) this.#nativeError = true;
      this.#fail(new TransportError(this.#opened ? "closed" : "connection",
        this.#opened ? "WebSocket stream disconnected" : "WebSocket connection failed",
        { cause: this.#upgrade.failure ?? (event as Event & { error?: unknown }).error }), true);
    });
    this.#socket.addEventListener("close", (event) => this.#fail(new TransportError("closed",
      "WebSocket connection closed", { closeCode: event.code, cause: this.#upgrade.failure }), true));
    this.#socket.addEventListener("message", (event) => {
      if (this.#failure !== undefined) return;
      this.#lastMessageAt = performance.now();
      this.#receivedEvents++;
      const chars = typeof event.data === "string" ? event.data.length : 0;
      this.#receivedChars += chars;
      this.#largestEventChars = Math.max(this.#largestEventChars, chars);
      if (typeof event.data !== "string" || event.data.length > MAX_SSE_EVENT_CHARS) {
        this.#fail(new TransportError("event-limit", "WebSocket event exceeded its text message limit"));
        return;
      }
      if (this.#queue.length >= 4_096 || this.#queuedChars + event.data.length > 4_000_000) {
        this.#fail(new TransportError("queue-limit", "WebSocket event queue exceeded its limit"));
        return;
      }
      this.#queue.push(event.data);
      this.#queuedChars += event.data.length;
      this.#wake?.();
    });
  }

  get ready(): boolean { return this.#failure === undefined && this.#socket.readyState === WebSocket.OPEN; }

  get statistics(): Pick<TransportObservation, "receivedEvents" | "receivedChars" | "largestEventChars" |
    "connectionAgeMs" | "lastMessageAgeMs" | "socketReadEnded" | "nativeWebSocketError"> {
    const now = this.#failedAt ?? performance.now();
    return { receivedEvents: this.#receivedEvents, receivedChars: this.#receivedChars,
      largestEventChars: this.#largestEventChars,
      ...(this.#openedAt === undefined ? {} : { connectionAgeMs: Math.round(now - this.#openedAt) }),
      ...(this.#lastMessageAt === undefined ? {} : { lastMessageAgeMs: Math.round(now - this.#lastMessageAt) }),
      socketReadEnded: this.#upgrade.readEnded, nativeWebSocketError: this.#nativeError };
  }

  async connected(signal?: AbortSignal): Promise<void> {
    signal?.throwIfAborted();
    if (!this.ready && this.#failure === undefined) await this.#wait(5_000, signal);
    if (this.#failure !== undefined) throw this.#failure;
    if (!this.ready) throw new TransportError("connection", "WebSocket did not open");
  }

  send(text: string): void {
    if (!this.ready || this.#queue.length !== 0) throw new TransportError("not-ready", "WebSocket is not ready for a request");
    this.#receivedEvents = 0;
    this.#receivedChars = 0;
    this.#largestEventChars = 0;
    this.#lastMessageAt = undefined;
    this.#socket.send(text);
  }

  async next(milliseconds: number, signal?: AbortSignal,
    timeout = () => new TransportError("idle-timeout", "WebSocket stream timed out")): Promise<string> {
    signal?.throwIfAborted();
    if (this.#queue.length === 0 && this.#failure === undefined) await this.#wait(milliseconds, signal, timeout);
    // A peer can close immediately after its terminal event: deliver buffered
    // events first so a fully received response does not become a false failure.
    const value = this.#queue.shift();
    if (value !== undefined) { this.#queuedChars -= value.length; return value; }
    throw this.#failure ?? new TransportError("closed", "WebSocket stream ended without an event");
  }

  close(): void { this.#fail(new Error("WebSocket scope closed")); }

  #fail(error: Error, keepQueued = false): void {
    const first = this.#failure === undefined;
    if (first) this.#failedAt = performance.now();
    this.#failure ??= error;
    if (!keepQueued) { this.#queue.length = 0; this.#queuedChars = 0; }
    // Node 22 can synchronously dispatch another error from close() while an
    // upgrade is failing. Record settlement before invoking the native close.
    if (first) this.#socket.close();
    this.#upgrade.destroy();
    this.#wake?.();
  }

  async #wait(milliseconds: number, signal?: AbortSignal,
    timeout = () => new TransportError("connection", "WebSocket connection timed out")): Promise<void> {
    let timer: NodeJS.Timeout | undefined;
    const abort = (): void => this.#wake?.();
    try {
      await new Promise<void>((resolve) => {
        this.#wake = resolve;
        timer = setTimeout(() => this.#fail(timeout()), milliseconds);
        signal?.addEventListener("abort", abort, { once: true });
        if (signal?.aborted) resolve();
      });
      signal?.throwIfAborted();
    } finally {
      clearTimeout(timer);
      this.#wake = undefined;
      signal?.removeEventListener("abort", abort);
    }
  }
}
