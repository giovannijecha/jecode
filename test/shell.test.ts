import { test } from "node:test";
import assert from "node:assert/strict";
import { runCommand } from "../src/tools/shell.ts";
import { runTool } from "../src/tools/index.ts";

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
  assert.equal(result.isError, true);
});

test("marks a non-zero command as an error result", async () => {
  const result = await runCommand.run(
    { command: "node -e \"process.exit(7)\"" },
    { root },
  );
  assert.equal(result.summary, "exit 7");
  assert.equal(result.isError, true);
});

test("propagates a failed command as a model-visible tool error", async () => {
  const result = await runTool(
    runCommand,
    {
      kind: "tool_call",
      id: "failed-command",
      name: "run_command",
      input: { command: "node -e \"process.exit(7)\"" },
    },
    { root },
  );

  assert.equal(result.result.isError, true);
  assert.match(result.result.output, /exit 7/);
});

test("commands that read stdin receive EOF instead of hanging", { timeout: 5_000 }, async () => {
  const result = await runCommand.run(
    {
      command:
        "node -e \"process.stdin.resume(); process.stdin.on('end', () => process.stdout.write('eof'))\"",
      timeout_ms: 1_000,
    },
    { root },
  );
  assert.equal(result.summary, "exit 0");
  assert.match(result.output, /^eof/);
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

test("does not pass credential-like variables to a shell command", async (context) => {
  const name = "JECODE_REVIEW_API_KEY";
  const before = process.env[name];
  process.env[name] = "fixture-child-environment-secret";
  context.after(() => restoreEnvironment(name, before));

  const result = await runCommand.run(
    { command: `node -e "process.stdout.write(process.env.${name} ?? 'missing')"` },
    { root },
  );

  assert.match(result.output, /^missing/);
  assert.doesNotMatch(result.output, /fixture-child-environment-secret/);
});

test("does not pass a credential-bearing connection URL to a shell command", async (context) => {
  const name = "JECODE_REVIEW_DATABASE_URL";
  const before = process.env[name];
  process.env[name] = "postgres://fixture-user:fixture-password@localhost/database";
  context.after(() => restoreEnvironment(name, before));

  const result = await runCommand.run(
    { command: `node -e "process.stdout.write(process.env.${name} ?? 'missing')"` },
    { root },
  );

  assert.match(result.output, /^missing/);
  assert.doesNotMatch(result.output, /fixture-password/);
});

test("redacts a known credential before shell output leaves the tool", async (context) => {
  const secret = "fixture-visible-credential-8173";
  const keyBefore = process.env["OPENAI_API_KEY"];
  const visibleBefore = process.env["JECODE_REVIEW_VISIBLE"];
  process.env["OPENAI_API_KEY"] = secret;
  process.env["JECODE_REVIEW_VISIBLE"] = secret;
  context.after(() => {
    restoreEnvironment("OPENAI_API_KEY", keyBefore);
    restoreEnvironment("JECODE_REVIEW_VISIBLE", visibleBefore);
  });

  const result = await runCommand.run(
    { command: "node -e \"process.stdout.write(process.env.JECODE_REVIEW_VISIBLE ?? '')\"" },
    { root },
  );

  assert.match(result.output, /\[credential redacted\]/);
  assert.doesNotMatch(result.output, /fixture-visible-credential-8173/);
});

test("live command snapshots are bounded and redacted before the TUI sees them", async (context) => {
  const secret = "fixture-live-credential-91827";
  const keyBefore = process.env["OPENAI_API_KEY"];
  const visibleBefore = process.env["JECODE_REVIEW_VISIBLE"];
  process.env["OPENAI_API_KEY"] = secret;
  process.env["JECODE_REVIEW_VISIBLE"] = secret;
  context.after(() => {
    restoreEnvironment("OPENAI_API_KEY", keyBefore);
    restoreEnvironment("JECODE_REVIEW_VISIBLE", visibleBefore);
  });

  const snapshots: string[] = [];
  await runCommand.run(
    {
      command:
        "node -e \"process.stdout.write('start\\n' + (process.env.JECODE_REVIEW_VISIBLE ?? '') + '\\n' + 'x'.repeat(40000))\"",
    },
    { root, onOutput: (output) => snapshots.push(output) },
  );

  assert.ok(snapshots.length > 0);
  assert.ok(snapshots.some((output) => output.includes("[credential redacted]")));
  assert.ok(snapshots.every((output) => !output.includes(secret)));
  assert.ok(snapshots.every((output) => output.length < 31_000));
});

test("redacts a credential before a bounded head-tail output split", async (context) => {
  const secret = "fixture-boundary-credential-762918";
  const keyBefore = process.env["OPENAI_API_KEY"];
  const visibleBefore = process.env["JECODE_REVIEW_VISIBLE"];
  process.env["OPENAI_API_KEY"] = secret;
  process.env["JECODE_REVIEW_VISIBLE"] = secret;
  context.after(() => {
    restoreEnvironment("OPENAI_API_KEY", keyBefore);
    restoreEnvironment("JECODE_REVIEW_VISIBLE", visibleBefore);
  });

  const result = await runCommand.run(
    {
      command:
        "node -e \"const value = process.env.JECODE_REVIEW_VISIBLE ?? ''; " +
        "process.stdout.write('A'.repeat(14990) + value + 'B'.repeat(20000))\"",
    },
    { root },
  );

  assert.doesNotMatch(result.output, /fixture-boundary/);
  assert.doesNotMatch(result.output, /credential-762918/);
  assert.match(result.output, /characters cut/);
});

function restoreEnvironment(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}
