import { test } from "node:test";
import assert from "node:assert/strict";
import { runCommand } from "../src/tools/shell.ts";

const root = process.cwd();

test("rejects a command before spawning when its signal is already aborted", async () => {
  const control = new AbortController();
  control.abort(new Error("cancelled by test"));
  await assert.rejects(
    runCommand.run({ command: "node -e \"process.exit(0)\"" }, { root, signal: control.signal }),
    /cancelled by test/,
  );
});

test("kills a running command when the turn is aborted", { timeout: 5_000 }, async () => {
  const control = new AbortController();
  const running = runCommand.run(
    { command: "node -e \"setInterval(() => {}, 1000)\"", timeout_ms: 4_000 },
    { root, signal: control.signal },
  );
  setTimeout(() => control.abort(new Error("cancelled by test")), 50);
  await assert.rejects(running, /cancelled by test/);
});

test("times out a process tree and reports the timeout", { timeout: 5_000 }, async () => {
  const result = await runCommand.run(
    { command: "node -e \"setInterval(() => {}, 1000)\"", timeout_ms: 50 },
    { root },
  );
  assert.equal(result.summary, "timed out after 50ms");
  assert.match(result.output, /timed out after 50ms/);
});

test("keeps bounded head and tail output", async () => {
  const result = await runCommand.run(
    { command: "node -e \"process.stdout.write('A'.repeat(20000) + 'B'.repeat(20000))\"" },
    { root },
  );
  assert.match(result.output, /^A+/);
  assert.match(result.output, /characters cut/);
  assert.match(result.output, /B+\n\[exit 0\]$/);
  assert.ok(result.output.length < 31_000);
});

test("rejects a non-positive timeout", async () => {
  await assert.rejects(
    runCommand.run({ command: "node -e \"process.exit(0)\"", timeout_ms: 0 }, { root }),
    /must be a positive integer/,
  );
});
