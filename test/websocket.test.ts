import { test } from "node:test";
import assert from "node:assert/strict";
import { SocketChannel } from "../src/providers/websocket.ts";
import { responsesServer, socketText } from "../dev/test-support/responses-server.ts";

test("idle socket reads time out and close the transport", async (t) => {
  const server = await responsesServer({ message() {} });
  const channel = new SocketChannel(server.url.replace("http:", "ws:"), {});
  t.after(async () => { channel.close(); await server.close(); });
  await channel.connected();
  await assert.rejects(channel.next(5), /timed out/);
  assert.equal(channel.ready, false);
  assert.throws(() => channel.send("{}"), /not ready/);
});

test("cancellation while connecting stops the scope before any generation", async (t) => {
  const control = new AbortController();
  const server = await responsesServer({
    upgrade() { control.abort(new Error("cancel connect")); return false; },
    message() { assert.fail("no generation before open"); },
  });
  const channel = new SocketChannel(server.url.replace("http:", "ws:"), {});
  t.after(async () => { channel.close(); await server.close(); });
  await assert.rejects(channel.connected(control.signal), /cancel connect/);
  channel.close();
  channel.close();
  assert.equal(channel.ready, false);
});

test("queued socket data is bounded even when its consumer stops reading", { timeout: 10_000 }, async () => {
  for (const payloads of [Array(4100).fill("{}") as string[], Array(5).fill("x".repeat(900_000)) as string[]]) {
    let peerClosed!: () => void;
    const closed = new Promise<void>(resolve => { peerClosed = resolve; });
    const server = await responsesServer({ message(_body, socket) {
      socket.on("close", peerClosed);
      for (const text of payloads) socketText(socket, text);
    } });
    const channel = new SocketChannel(server.url.replace("http:", "ws:"), {});
    try {
      await channel.connected();
      channel.send("{}");
      await closed;
      await assert.rejects(channel.next(1000), /queue exceeded/);
    } finally { channel.close(); await server.close(); }
  }
});

test("authenticated upgrades reject redirects without forwarding headers", async (t) => {
  const destination = await responsesServer({ message() { assert.fail("redirect destination must not generate"); } });
  const origin = await responsesServer({ message() {}, upgrade(_request, socket) {
    socket.end(`HTTP/1.1 302 Found\r\nLocation: ${destination.url}\r\nContent-Length: 0\r\n\r\n`);
    return false;
  } });
  const channel = new SocketChannel(origin.url.replace("http:", "ws:"), { authorization: "Bearer fixture" });
  t.after(async () => { channel.close(); await origin.close(); await destination.close(); });
  await assert.rejects(channel.connected(), /WebSocket/);
  assert.deepEqual(destination.counts(), { upgrades: 0, posts: 0 });
});

test("closing a scope ends TCP even when the peer ignores close frames", { timeout: 2000 }, async (t) => {
  let peerEnded!: () => void;
  const ended = new Promise<void>(resolve => { peerEnded = resolve; });
  const server = await responsesServer({ ignoreClose: true, message() {}, upgrade(_request, socket) {
    socket.on("end", peerEnded);
    return true;
  } });
  const channel = new SocketChannel(server.url.replace("http:", "ws:"), {});
  t.after(async () => { channel.close(); await server.close(); });
  await channel.connected();
  channel.close();
  await ended;
  assert.equal(channel.ready, false);
});

test("reused sockets preserve fragmented UTF-8 messages with interleaved ping frames", { timeout: 5000 }, async (t) => {
  const expected = JSON.stringify({ text: "résultat 日本語 🦎".repeat(20) });
  const payload = Buffer.from(expected);
  const server = await responsesServer({ message(_body, socket) {
    const frames: Buffer[] = [];
    for (let offset = 0; offset < payload.length; offset += 17) {
      const fragment = payload.subarray(offset, offset + 17);
      const final = offset + 17 >= payload.length;
      frames.push(Buffer.concat([Buffer.from([(final ? 0x80 : 0) | (offset === 0 ? 1 : 0), fragment.length]), fragment]));
      frames.push(Buffer.from([0x89, 1, 42]));
    }
    // TCP chunks deliberately split both frame headers and UTF-8 code points.
    const bytes = Buffer.concat(frames);
    socket.write(bytes.subarray(0, 3));
    socket.write(bytes.subarray(3, 71));
    socket.write(bytes.subarray(71));
  } });
  const channel = new SocketChannel(server.url.replace("http:", "ws:"), {});
  t.after(async () => { channel.close(); await server.close(); });
  await channel.connected();
  for (let index = 0; index < 32; index++) {
    channel.send("{}");
    assert.equal(await channel.next(1000), expected);
    assert.equal(channel.statistics.receivedEvents, 1);
  }
  assert.equal(server.counts().upgrades, 1);
});
