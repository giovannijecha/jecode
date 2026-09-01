import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
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

test("rejects a timeout larger than Node can schedule", async () => {
  await assert.rejects(
    runCommand.run(
      { command: "node -e \"process.exit(0)\"", timeout_ms: 2_147_483_648 },
      { root },
    ),
    /must be at most 2147483647ms/,
  );
});

test("does not wait indefinitely for a descendant holding output pipes", { timeout: 5_000 }, async () => {
  const source = [
    "const { spawn } = require('node:child_process');",
    "const child = spawn(process.execPath, ['-e', 'setTimeout(() => {}, 2000)'],",
    "  { detached: true, stdio: 'inherit', windowsHide: true });",
    "child.unref();",
  ].join("\n");
  const encoded = Buffer.from(source).toString("base64");
  const started = Date.now();

  const result = await runCommand.run(
    { command: `"${process.execPath}" -e "eval(Buffer.from('${encoded}','base64').toString())"` },
    { root },
  );

  assert.equal(result.summary, "exit 0");
  assert.ok(Date.now() - started < 1_000, "command waited for a detached descendant's pipe");
});

test("a timeout force-kills a descendant that ignores graceful termination", {
  skip: process.platform === "win32",
  timeout: 5_000,
}, async () => {
  const area = await mkdtemp(path.join(tmpdir(), "jecode-shell-tree-"));
  const pidFile = path.join(area, "pid");
  const source = [
    "const fs = require('node:fs');",
    `fs.writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));`,
    "process.on('SIGTERM', () => {});",
    "setInterval(() => {}, 1000);",
  ].join("\n");
  const encoded = Buffer.from(source).toString("base64");
  let pid: number | undefined;

  try {
    const result = await runCommand.run(
      {
        command: `"${process.execPath}" -e "eval(Buffer.from('${encoded}','base64').toString())"`,
        timeout_ms: 300,
      },
      { root },
    );
    pid = Number(await readFile(pidFile, "utf8"));

    assert.equal(result.summary, "timed out after 300ms");
    await waitForExit(pid);
    assert.equal(alive(pid), false, `descendant ${pid} survived the timeout`);
  } finally {
    if (pid !== undefined && alive(pid)) process.kill(pid, "SIGKILL");
    await rm(area, { recursive: true, force: true });
  }
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

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function waitForExit(pid: number): Promise<void> {
  const deadline = Date.now() + 1_000;
  while (alive(pid) && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}
