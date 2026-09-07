import { test } from "node:test";
import assert from "node:assert/strict";
import type { SessionPersistence } from "../src/sessions/runtime.ts";
import { runApp } from "../src/tui/app.ts";
import { session } from "../dev/test-support/app.ts";
import { virtualScreen } from "../dev/test-support/app-harness.ts";

for (const scenario of ["already stopped", "screen entry failed", "first paint failed"] as const) {
  test(`resume startup releases pending work when ${scenario}`, { timeout: 2_000 }, async () => {
    const current = session();
    let closed = 0;
    let entered = 0;
    current.persistence = {
      close: async () => { closed++; },
    } as SessionPersistence;
    current.resume = {
      candidates: [{
        id: "saved-1",
        createdAt: "2026-09-01T10:00:00.000Z",
        updatedAt: "2026-09-01T10:01:00.000Z",
        turns: 1,
        preview: "saved question",
        active: false,
      }],
      open: async () => { assert.fail("failed startup must not open a session"); },
    };
    const shutdown = new AbortController();
    const failure = new Error(`fixture: ${scenario}`);
    const harness = virtualScreen(40);
    harness.environment.screen.enter = () => {
      entered++;
      if (scenario === "screen entry failed") throw failure;
    };
    if (scenario === "first paint failed") {
      harness.environment.paint.paint = () => { throw failure; };
    }
    if (scenario === "already stopped") shutdown.abort(failure);

    const running = runApp(current, process.cwd(), {
      ...harness.environment,
      shutdownSignal: shutdown.signal,
    });
    if (scenario === "already stopped") await running;
    else await assert.rejects(running, (error: unknown) => error === failure);

    assert.equal(entered, scenario === "already stopped" ? 0 : 1);
    assert.equal(closed, 1);
    assert.equal(harness.left(), true);
  });
}
