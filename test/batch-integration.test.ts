import { test } from "node:test";
import assert from "node:assert/strict";
import { runBatch } from "../src/batch.ts";
import type { Message, Provider, SendRequest } from "../src/types.ts";
import { MAX_PROMPT_CODE_UNITS } from "../src/input-boundary.ts";
import { provider, session, input } from "../dev/test-support/app.ts";
import { waitFor } from "../dev/test-support/app-harness.ts";

test("batch mode carries input through the controller, renderer, commands, and exit", async () => {
  const output: string[] = [];
  const current = session();

  await runBatch(current, {
    lines: input("hello", "/help", "/exit", "ignored"),
    width: 60,
    write: (text) => output.push(text),
  });

  const shown = output.join("");
  assert.match(shown, /> hello/);
  assert.match(shown, /Hello from fake\./);
  assert.match(shown, /interactive help needs the TUI/);
  assert.doesNotMatch(shown, /ignored/);
  assert.equal(current.conversation.history.length, 2);
});

test("batch mode rejects an oversized line before echo, history, or provider use", async () => {
  let requests = 0;
  const counted: Provider = {
    ...provider(),
    async send(request): Promise<Message> {
      requests++;
      return provider().send(request);
    },
  };
  const current = session(counted);
  const output: string[] = [];

  await assert.rejects(
    runBatch(current, {
      lines: input("x".repeat(MAX_PROMPT_CODE_UNITS + 1)),
      write: (text) => output.push(text),
    }),
    /Prompt cannot exceed 1,048,576 UTF-16 code units/,
  );

  assert.equal(requests, 0);
  assert.equal(output.join(""), "");
  assert.deepEqual(current.conversation.history, []);
});

test("batch mode propagates provider failures outside the transcript", async () => {
  const failed: Provider = {
    ...provider(),
    async send(): Promise<Message> {
      throw Object.assign(new Error("fixture provider failed"), {
        body: '{"error":{"message":"requested model is unavailable"}}',
      });
    },
  };
  const output: string[] = [];

  await assert.rejects(
    runBatch(session(failed), {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /fixture provider failed · requested model is unavailable/,
  );

  const shown = output.join("");
  assert.match(shown, /> hello/);
  assert.doesNotMatch(shown, /fixture provider failed|requested model is unavailable/);
});

test("batch mode discards an incomplete streamed answer when the provider fails", async () => {
  const failed: Provider = {
    ...provider(),
    async send(request: SendRequest): Promise<Message> {
      request.onStream?.({ kind: "text", text: "partial answer" });
      throw new Error("stream failed");
    },
  };
  const output: string[] = [];

  await assert.rejects(
    runBatch(session(failed), {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /stream failed/,
  );

  assert.doesNotMatch(output.join(""), /partial answer|stream failed/);
});

test("batch mode propagates an explicit model-request budget", async () => {
  let sends = 0;
  const looping: Provider = {
    ...provider(),
    async send(): Promise<Message> {
      sends++;
      return {
        role: "assistant",
        content: [{ kind: "tool_call", id: String(sends), name: "missing", input: {} }],
      };
    },
  };
  const current = session(looping);
  current.config.maxModelRequests = 2;
  const output: string[] = [];

  await assert.rejects(
    runBatch(current, {
      lines: input("hello"),
      width: 60,
      write: (text) => output.push(text),
    }),
    /stopped after 2 model requests/,
  );

  assert.equal(sends, 2);
  assert.doesNotMatch(output.join(""), /stopped after/);
});

test("batch process cancellation reaches the active provider request", async () => {
  let providerSignal: AbortSignal | undefined;
  const waiting: Provider = {
    ...provider(),
    send(request): Promise<Message> {
      providerSignal = request.signal;
      return new Promise((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), {
          once: true,
        });
      });
    },
  };
  const shutdown = new AbortController();
  const running = runBatch(session(waiting), {
    lines: input("wait"),
    signal: shutdown.signal,
    write: () => {},
  });

  await waitFor(() => providerSignal !== undefined, "batch provider request");
  const reason = new Error("received SIGTERM");
  shutdown.abort(reason);

  await assert.rejects(running, (error) => error === reason);
  assert.equal(providerSignal?.aborted, true);
});

test("batch process cancellation interrupts an idle input wait", async () => {
  let waiting = false;
  const lines: AsyncIterable<string> = {
    [Symbol.asyncIterator]() {
      return {
        next() {
          waiting = true;
          return new Promise<IteratorResult<string>>(() => {});
        },
      };
    },
  };
  const shutdown = new AbortController();
  const running = runBatch(session(), { lines, signal: shutdown.signal, write: () => {} });

  await waitFor(() => waiting, "batch input wait");
  const reason = new Error("received SIGTERM");
  shutdown.abort(reason);

  await assert.rejects(running, (error) => error === reason);
});
