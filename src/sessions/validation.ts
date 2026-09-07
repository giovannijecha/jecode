// Share only checks requested before filesystem validation starts. A running
// or completed check cannot validate a later read/write boundary.

import { setImmediate } from "node:timers/promises";

export function queuedValidation(check: () => Promise<void>): () => Promise<void> {
  let queued: Promise<void> | undefined;
  return () => {
    queued ??= setImmediate().then(async () => {
      // Clear before invoking the check: callers arriving while IO is pending
      // need a fresh observation, even if this check eventually succeeds.
      queued = undefined;
      await check();
    });
    return queued;
  };
}
