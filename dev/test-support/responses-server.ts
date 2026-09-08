// Loopback-only transport fixture. The runtime uses Node's WebSocket parser;
// this small server decodes masked client frames solely for protocol tests.
import { createServer } from "node:http";
import type { IncomingMessage, ServerResponse } from "node:http";
import { createHash } from "node:crypto";
import type { Duplex } from "node:stream";

export async function responsesServer(options: {
  message(body: Record<string, unknown>, socket: Duplex): void;
  http?(request: IncomingMessage, response: ServerResponse): void;
  upgrade?(request: IncomingMessage, socket: Duplex): boolean;
  ignoreClose?: boolean;
}) {
  const connections = new Set<Duplex>();
  let upgrades = 0;
  let posts = 0;
  const server = createServer((request, response) => {
    posts++;
    if (options.http !== undefined) options.http(request, response);
    else { response.writeHead(500); response.end(); }
  });
  server.on("connection", (socket) => {
    connections.add(socket);
    socket.on("error", () => undefined);
    socket.on("close", () => connections.delete(socket));
  });
  server.on("upgrade", (request, socket, head) => {
    upgrades++;
    if (options.upgrade?.(request, socket) === false) return;
    const accept = createHash("sha1")
      .update(`${request.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64");
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    let pending = Buffer.from(head);
    const consume = (chunk: Buffer): void => {
      pending = Buffer.concat([pending, chunk]);
      while (pending.length >= 2) {
        const opcode = pending[0]! & 15;
        const size = pending[1]! & 127;
        const offset = size < 126 ? 2 : size === 126 ? 4 : 10;
        if (pending.length < offset + 4) return;
        const length = size < 126 ? size : size === 126 ? pending.readUInt16BE(2) : Number(pending.readBigUInt64BE(2));
        if (length > 16_000_000) { socket.destroy(); return; }
        if (pending.length < offset + 4 + length) return;
        const mask = pending.subarray(offset, offset + 4);
        const payload = Buffer.from(pending.subarray(offset + 4, offset + 4 + length));
        pending = pending.subarray(offset + 4 + length);
        for (let index = 0; index < payload.length; index++) payload[index] = payload[index]! ^ mask[index % 4]!;
        if (opcode === 8) {
          if (!options.ignoreClose) socket.end(Buffer.from([0x88, 0]));
          return;
        }
        if (opcode === 1) options.message(JSON.parse(payload.toString()) as Record<string, unknown>, socket);
      }
    };
    socket.on("data", consume);
    if (head.length > 0) consume(Buffer.alloc(0));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("fixture did not bind");
  return {
    url: `http://127.0.0.1:${address.port}/responses`,
    counts: () => ({ upgrades, posts }),
    async close() {
      for (const socket of connections) socket.destroy();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}

export function socketJson(socket: Duplex, value: unknown): void {
  socketText(socket, JSON.stringify(value));
}

export function socketText(socket: Duplex, text: string): void {
  const payload = Buffer.from(text);
  const prefix = Buffer.alloc(payload.length < 126 ? 2 : payload.length <= 65_535 ? 4 : 10);
  prefix[0] = 0x81;
  prefix[1] = prefix.length === 2 ? payload.length : prefix.length === 4 ? 126 : 127;
  if (prefix.length === 4) prefix.writeUInt16BE(payload.length, 2);
  if (prefix.length === 10) prefix.writeBigUInt64BE(BigInt(payload.length), 2);
  socket.write(Buffer.concat([prefix, payload]));
}

export function completed(id = "response-fixture", text = "done") {
  return { type: "response.completed", response: { id, status: "completed",
    output: [{ type: "message", role: "assistant", content: [{ type: "output_text", text }] }],
    usage: { input_tokens: 100, output_tokens: 5 } } };
}
