import { test } from "node:test";
import assert from "node:assert/strict";
import { runBatch } from "../src/batch.ts";
import { ConversationTree } from "../src/conversation.ts";
import type { Message, Provider } from "../src/types.ts";
import { provider, session, input } from "../dev/test-support/app.ts";
import { waitFor } from "../dev/test-support/app-harness.ts";

for (const phase of ["metadata lookup", "automatic compaction"] as const) {
  test(`batch cancellation interrupts ${phase} before normal generation`, async () => {
    const shutdown = new AbortController();
    const reason = new Error("received SIGTERM");
    let started = false;
    let requestSignal: AbortSignal | undefined;
    let release = (): void => {};
    let normalRequests = 0;

    const waitForCancellation = (signal: AbortSignal | undefined): Promise<never> => {
      requestSignal = signal;
      return new Promise((_resolve, reject) => {
        const abort = (): void => reject(signal?.reason);
        release = () => {
          signal?.removeEventListener("abort", abort);
          reject(reason);
        };
        signal?.addEventListener("abort", abort, { once: true });
        started = true;
      });
    };
    const waiting: Provider = {
      ...provider(),
      async contextWindow(_model, signal) {
        if (phase === "metadata lookup") return waitForCancellation(signal);
        return { tokens: 64_000 };
      },
      async send(request): Promise<Message> {
        if (request.system.includes("durable working memory")) {
          return waitForCancellation(request.signal);
        }
        normalRequests++;
        return provider().send(request);
      },
    };
    const current = session(waiting);
    if (phase === "automatic compaction") {
      current.conversation = ConversationTree.empty().commit({
        parentId: 0,
        createdAt: "2026-09-01T10:00:00.000Z",
        identity: { providerId: "fake", model: "fake-1", effort: "high" },
        messages: [
          { role: "user", content: [{ kind: "text", text: "old context ".repeat(10_000) }] },
          { role: "assistant", content: [{ kind: "text", text: "Old answer." }] },
        ],
        blocks: [],
      }, "completed");
      current.usage.lastInputTokens = 65_000;
    }
    const before = current.conversation.history;
    const running = runBatch(current, {
      lines: input("continue"),
      signal: shutdown.signal,
      write: () => {},
    });
    const rejected = assert.rejects(running, (error) => error === reason);
    void rejected.catch(() => {});

    try {
      await waitFor(() => started, `batch ${phase}`);
      shutdown.abort(reason);

      assert.equal(requestSignal?.aborted, true, `${phase} must receive cancellation`);
      await rejected;
      assert.equal(normalRequests, 0);
      assert.deepEqual(current.conversation.history, before);
    } finally {
      release();
      await rejected;
    }
  });
}
