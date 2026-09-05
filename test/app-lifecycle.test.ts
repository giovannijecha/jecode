import { test } from "node:test";
import assert from "node:assert/strict";
import { ConversationTree } from "../src/conversation.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import type { Message, Provider } from "../src/types.ts";
import type { Painter } from "../src/tui/frame.ts";
import { runApp } from "../src/tui/app.ts";
import type { AppScreen } from "../src/tui/app.ts";
import { provider, session } from "../dev/test-support/app.ts";
import { virtualScreen, waitFor, delay } from "../dev/test-support/app-harness.ts";

test("the TUI owns and restores a screen around a real /exit interaction", async () => {
  let feed: ((chunk: string) => void) | undefined;
  let entered = false;
  let left = false;
  let inputStopped = false;
  let resizeStopped = false;
  let outputStopped = false;
  let outputReady = () => {};
  const frames: string[][] = [];

  const screen: AppScreen = {
    size: () => ({ rows: 18, cols: 70 }),
    enter: () => {
      entered = true;
    },
    leave: () => {
      left = true;
    },
    setReducedMotion: () => {},
    onResize: () => () => {
      resizeStopped = true;
    },
    onInput: (handler) => {
      feed = handler;
      return () => {
        inputStopped = true;
      };
    },
  };
  const paint: Painter = {
    paint: (rows) => frames.push([...rows]),
    invalidate: () => {},
    onReady: (handler) => {
      outputReady = handler;
      return () => { outputStopped = true; };
    },
  };

  const running = runApp(session(), process.cwd(), { screen, paint });
  while (feed === undefined) await new Promise<void>((resolve) => setImmediate(resolve));
  const firstFrames = frames.length;
  outputReady();
  await waitFor(() => frames.length > firstFrames, "redraw after output drain");
  feed("/exit\r");
  await running;

  assert.equal(entered, true);
  assert.equal(left, true);
  assert.equal(inputStopped, true);
  assert.equal(resizeStopped, true);
  assert.equal(outputStopped, true);
  const finalFrames = frames.length;
  outputReady();
  await delay(30);
  assert.equal(frames.length, finalFrames);
  assert.ok(frames.length > 0);
  assert.ok(frames.every((frame) => frame.length === 18));
});

test("the TUI completes a large transcript reflow across scheduled frames", async () => {
  const current = session();
  const blocks = Array.from(
    { length: 300 },
    (_, index) => ({ kind: "answer" as const, text: `answer ${index}` }),
  );
  current.conversation = ConversationTree.empty().commit({
    parentId: 0,
    createdAt: "2026-09-02T10:00:00.000Z",
    identity: { providerId: "fake", model: "fake-1", effort: "high" },
    messages: [
      { role: "user", content: [{ kind: "text", text: "large transcript" }] },
      { role: "assistant", content: [{ kind: "text", text: "complete" }] },
    ],
    blocks,
  }, "completed");
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  await waitFor(() => harness.frames.length >= 3, "incremental transcript reflow");
  assert.match((harness.frames[0] ?? []).join("\n"), /answer 299/);
  assert.ok(harness.frames.every((frame) => frame.length === 18));
  const settledFrames = harness.frames.length;
  await delay(40);
  assert.equal(harness.frames.length, settledFrames);

  feed("/exit\r");
  await running;
});

test("initial and scheduled paint failures restore every TUI owner", async () => {
  for (const failureAt of [1, 2]) {
    let feed: ((chunk: string) => void) | undefined;
    let paints = 0;
    let left = 0;
    let inputStopped = 0;
    let resizeStopped = 0;
    let persistenceClosed = 0;
    const current = session();
    current.persistence = {
      close: async () => {
        persistenceClosed++;
      },
    } as SessionPersistence;
    const screen: AppScreen = {
      size: () => ({ rows: 18, cols: 70 }),
      enter: () => {},
      leave: () => {
        left++;
      },
      setReducedMotion: () => {},
      onResize: () => () => {
        resizeStopped++;
      },
      onInput: (handler) => {
        feed = handler;
        return () => {
          inputStopped++;
        };
      },
    };
    const paint: Painter = {
      paint: () => {
        paints++;
        if (paints === failureAt) throw new Error("fixture paint failed");
      },
      invalidate: () => {},
    };

    const running = runApp(current, process.cwd(), { screen, paint });
    const rejected = assert.rejects(running, /fixture paint failed/);
    while (feed === undefined) await new Promise<void>((resolve) => setImmediate(resolve));
    if (failureAt === 2) feed("x");
    await rejected;

    assert.equal(left, 1);
    assert.equal(inputStopped, 1);
    assert.equal(resizeStopped, 1);
    assert.equal(persistenceClosed, 1);
  }
});

test("a fatal paint failure aborts active work before persistence closes", async () => {
  let started = (): void => {};
  const providerStarted = new Promise<void>((resolve) => {
    started = resolve;
  });
  let aborted = false;
  let providerSettled = false;
  let persistenceClosedAfterSettlement = false;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      started();
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => {
          aborted = true;
          setTimeout(() => {
            providerSettled = true;
            reject(request.signal?.reason);
          }, 20);
        }, { once: true });
      });
    },
  };
  const current = session(waiting);
  current.persistence = {
    close: async () => {
      persistenceClosedAfterSettlement = providerSettled;
    },
  } as SessionPersistence;
  let paints = 0;
  const harness = virtualScreen();
  harness.environment.paint = {
    paint: (rows) => {
      paints++;
      if (paints === 2) throw new Error("fatal fixture paint failure");
      harness.frames.push([...rows]);
    },
    invalidate: () => {},
  };

  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();
  feed("wait\r");
  await providerStarted;
  await assert.rejects(running, /fatal fixture paint failure/);

  assert.equal(aborted, true);
  assert.equal(providerSettled, true);
  assert.equal(persistenceClosedAfterSettlement, true);
  assert.equal(harness.left(), true);
});

test("teardown releases every owner even when one cleanup callback throws", async () => {
  let feed: ((chunk: string) => void) | undefined;
  let resizeStopped = false;
  let left = false;
  let persistenceClosed = false;
  const current = session();
  current.persistence = {
    close: async () => {
      persistenceClosed = true;
    },
  } as SessionPersistence;
  const screen: AppScreen = {
    size: () => ({ rows: 18, cols: 70 }),
    enter: () => {},
    leave: () => {
      left = true;
    },
    setReducedMotion: () => {},
    onResize: () => () => {
      resizeStopped = true;
    },
    onInput: (handler) => {
      feed = handler;
      return () => {
        throw new Error("fixture input cleanup failed");
      };
    },
  };
  const paint: Painter = { paint: () => {}, invalidate: () => {} };

  const running = runApp(current, process.cwd(), { screen, paint });
  await waitFor(() => feed !== undefined, "TUI input handler");
  feed?.("/exit\r");
  await assert.rejects(running, /fixture input cleanup failed/);

  assert.equal(resizeStopped, true);
  assert.equal(left, true);
  assert.equal(persistenceClosed, true);
});

test("only the latest input chunk owns the escape grace timer", async () => {
  const harness = virtualScreen();
  const current = session();
  const running = runApp(current, process.cwd(), harness.environment);
  const feed = await harness.input();

  try {
    feed("draft");
    await delay(15);
    feed(String.fromCharCode(27));
    await delay(15);
    feed("[A");
    await delay(35);
    feed("\r");

    await waitFor(() => current.conversation.history.length === 2, "turn after split cursor sequence");
    const prompt = current.conversation.history[0]?.content[0];
    assert.equal(prompt?.kind === "text" ? prompt.text : undefined, "draft");
  } finally {
    feed("/exit\r");
    await running;
  }
});

test("escape cancels the active provider request before the TUI closes", async () => {
  let signal: AbortSignal | undefined;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      signal = request.signal;
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const harness = virtualScreen();
  const running = runApp(session(waiting), process.cwd(), harness.environment);
  const feed = await harness.input();

  feed("wait\r");
  await waitFor(() => signal !== undefined, "provider request");
  feed(String.fromCharCode(27));
  await waitFor(() => signal?.aborted === true, "provider cancellation");
  await waitFor(() => harness.frames.flat().join("\n").includes("[interrupted]"), "interrupted transcript");

  feed("/exit\r");
  await running;
  assert.equal(harness.left(), true);
});

test("process shutdown aborts one active TUI turn and waits for its settlement", async () => {
  let signal: AbortSignal | undefined;
  let checkpoints = 0;
  const waiting: Provider = {
    ...provider(),
    send: (request) => {
      signal = request.signal;
      return new Promise<Message>((_resolve, reject) => {
        request.signal?.addEventListener("abort", () => reject(request.signal?.reason), { once: true });
      });
    },
  };
  const current = session(waiting);
  current.persistence = {
    checkpoint: async () => {
      checkpoints++;
    },
    close: async () => {},
  } as unknown as SessionPersistence;
  const shutdown = new AbortController();
  const harness = virtualScreen();
  const running = runApp(current, process.cwd(), {
    ...harness.environment,
    shutdownSignal: shutdown.signal,
  });
  const feed = await harness.input();

  feed("wait\r");
  await waitFor(() => signal !== undefined, "provider request");
  shutdown.abort(new Error("received SIGTERM"));
  await running;

  assert.equal(signal?.aborted, true);
  assert.equal(current.conversation.activeNode?.settlement, "interrupted");
  assert.equal(checkpoints, 1);
  assert.equal(harness.left(), true);
});
