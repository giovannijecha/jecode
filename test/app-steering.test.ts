import { test } from "node:test";
import assert from "node:assert/strict";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import type { Message, Provider } from "../src/types.ts";
import { runApp } from "../src/tui/app.ts";
import { provider, session, messageText } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor, lastFooter, deferred } from "../dev/test-support/app-harness.ts";

test("the TUI steers an active response inside the same durable turn", async () => {
  const first = deferred();
  const requests: Message[][] = [];
  const steeringProvider: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      requests.push(structuredClone(request.messages));
      if (requests.length === 1) {
        await first.wait;
        request.onStream?.({ kind: "text", text: "First answer." });
        return { role: "assistant", content: [{ kind: "text", text: "First answer." }] };
      }
      request.onStream?.({ kind: "text", text: "Revised answer." });
      return { role: "assistant", content: [{ kind: "text", text: "Revised answer." }] };
    },
  };
  const current = session(steeringProvider);
  const harness = virtualScreen(100);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("initial request\r");
  await waitFor(() => requests.length === 1, "first provider request");
  await waitFor(() => lastFooter(harness).includes("enter to steer"), "steering hint");
  feed("change direction\r");
  await waitFor(() => lastFooter(harness).includes("1 queued"), "queued steering footer");
  first.release();

  await waitFor(() => requests.length === 2, "steered provider request");
  await waitFor(
    () => current.conversation.activeNode?.settlement === "completed",
    "completed steered turn",
  );
  assert.equal(current.conversation.nodes.length, 1);
  assert.deepEqual(
    current.conversation.history.flatMap((message) => message.content)
      .filter((block) => block.kind === "text")
      .map((block) => block.text),
    ["initial request", "First answer.", "change direction", "Revised answer."],
  );
  assert.match(JSON.stringify(current.conversation.transcript), /change direction/);

  feed("/exit\r");
  await running;
});

test("interruption restores steering that the model did not receive", async () => {
  const requests: Message[][] = [];
  const waiting: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      requests.push(structuredClone(request.messages));
      if (requests.length > 1) {
        return { role: "assistant", content: [{ kind: "text", text: "Recovered." }] };
      }
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const current = session(waiting);
  const harness = virtualScreen(100);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("wait\r");
  await waitFor(() => requests.length === 1, "waiting provider request");
  feed("recover this guidance\r");
  await waitFor(() => lastFooter(harness).includes("1 queued"), "queued guidance");
  feed(String.fromCharCode(27));
  await waitFor(
    () => current.conversation.activeNode?.settlement === "interrupted" &&
      !lastFooter(harness).includes("esc to interrupt"),
    "settled interruption",
  );
  await waitFor(
    () => (harness.frames.at(-1) ?? []).join("\n").includes("recover this guidance"),
    "restored composer guidance",
  );

  feed("\r");
  await waitFor(() => requests.length === 2, "retried guidance request");
  await waitFor(
    () => current.conversation.activeNode?.settlement === "completed",
    "completed recovered guidance",
  );
  assert.equal(messageText(requests[1]?.at(-1)), "recover this guidance");

  feed("/exit\r");
  await running;
});

test("a failed checkpoint returns the original prompt and pending steering", async () => {
  let requested = false;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      requested = true;
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const current = session(waiting);
  current.persistence = {
    checkpoint: async () => {
      throw new Error("fixture checkpoint failed");
    },
    close: async () => {},
  } as unknown as SessionPersistence;
  const harness = virtualScreen(100);
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("original prompt\r");
  await waitFor(() => requested, "provider request");
  feed("pending guidance\r");
  await waitFor(() => lastFooter(harness).includes("1 queued"), "queued guidance");
  feed(String.fromCharCode(27));
  await waitFor(
    () => lastFooter(harness).includes("fixture checkpoint failed"),
    "checkpoint failure",
  );

  const frame = (harness.frames.at(-1) ?? []).join("\n");
  assert.match(frame, /original prompt/);
  assert.match(frame, /pending guidance/);

  feed(String.fromCharCode(3));
  await running;
});
