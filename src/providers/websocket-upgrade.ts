// A dedicated upgrade dispatcher gives us ownership of the underlying socket.
// Native WebSocket still validates the handshake and owns all frame parsing.
// Its public close() only starts a handshake; destroy() must also work when a
// peer never acknowledges that close, without depending on private Node fields.
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import type { ClientRequest } from "node:http";
import type { Duplex } from "node:stream";

type UpgradeOptions = {
  origin: string | URL; path: string; method: string;
  headers: string[] | Record<string, string>;
};
type UpgradeHandler = {
  onConnect(abort: () => void): void;
  onUpgrade(status: number, headers: string[], socket: Duplex): void;
  onError(error: Error): void;
};

export class WebSocketUpgrade {
  #request: ClientRequest | undefined;
  #socket: Duplex | undefined;
  #closed = false;
  #failure: Error | undefined;
  #readEnded = false;

  get failure(): Error | undefined { return this.#failure; }
  get readEnded(): boolean { return this.#readEnded; }

  dispatch(options: UpgradeOptions, handler: UpgradeHandler): boolean {
    if (this.#closed || this.#request !== undefined) {
      handler.onError(new Error("WebSocket upgrade scope is not available"));
      return false;
    }
    const url = new URL(options.path, options.origin);
    if (!["http:", "https:"].includes(url.protocol) || options.method !== "GET") {
      handler.onError(new Error("Invalid WebSocket upgrade request"));
      return false;
    }
    const entries = Array.isArray(options.headers)
      ? options.headers.flatMap((value, index, all) => index % 2 === 0 ? [[value, all[index + 1]!]] : [])
      : Object.entries(options.headers);
    const headers = { ...Object.fromEntries(entries), connection: "Upgrade", upgrade: "websocket" };
    const open = url.protocol === "https:" ? httpsRequest : httpRequest;
    const request = open(url, { method: "GET", headers, agent: false });
    this.#request = request;
    request.on("error", (error) => { this.#failure ??= error; handler.onError(error); });
    request.on("response", (response) => {
      response.destroy();
      handler.onError(new Error("WebSocket upgrade rejected"));
    });
    request.on("upgrade", (response, socket, head) => {
      this.#socket = socket;
      // Observe EOF before handing frame processing to the native client. A
      // local destroy is not a peer EOF, and neither reveals why the peer left.
      socket.on("end", () => { this.#readEnded = true; });
      socket.on("error", (error) => { this.#failure ??= error; });
      if (this.#closed) { socket.destroy(); return; }
      if (head.length > 0) socket.unshift(head);
      handler.onUpgrade(response.statusCode ?? 0, response.rawHeaders, socket);
    });
    handler.onConnect(() => this.destroy());
    request.end();
    return true;
  }

  destroy(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#socket?.destroy();
    this.#request?.destroy();
  }
}
