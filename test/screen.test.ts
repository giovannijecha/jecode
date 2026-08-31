import { spawn } from "node:child_process";
import { test } from "node:test";
import assert from "node:assert/strict";

const SCREEN = new URL("../src/tui/screen.ts", import.meta.url).href;
const ALT_OFF = `${String.fromCharCode(27)}[?1049l`;

test("a fatal signal restores the terminal and keeps its conventional exit code", async () => {
  const result = await runScreenChild([
    setup(),
    `const { enter } = await import(${JSON.stringify(SCREEN)});`,
    "enter(true);",
    'process.emit("SIGTERM");',
  ].join("\n"));

  assert.equal(result.code, 143, result.stderr);
  assert.ok(result.stdout.includes(ALT_OFF));
});

test("a crash restores the terminal before its stack is printed", async () => {
  const result = await runScreenChild([
    setup(),
    `const { enter } = await import(${JSON.stringify(SCREEN)});`,
    "enter(true);",
    "process.on('uncaughtException', (error) => {",
    "  process.stdout.write(`STACK:${error.message}`);",
    "  process.exit(1);",
    "});",
    "throw new Error('fixture crash');",
  ].join("\n"));

  assert.equal(result.code, 1, result.stderr);
  assert.ok(result.stdout.includes(ALT_OFF));
  assert.ok(result.stdout.includes("STACK:fixture crash"));
  assert.ok(result.stdout.indexOf(ALT_OFF) < result.stdout.indexOf("STACK:fixture crash"));
});

function setup(): string {
  return [
    "Object.defineProperty(process.stdin, 'isTTY', { value: true });",
    "Object.defineProperty(process.stdout, 'isTTY', { value: true });",
    "Object.defineProperty(process.stdin, 'setRawMode', { value: () => process.stdin });",
  ].join("\n");
}

function runScreenChild(source: string): Promise<{
  code: number | null;
  stdout: string;
  stderr: string;
}> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["--input-type=module", "--eval", source], {
      cwd: process.cwd(),
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    child.once("error", reject);
    child.once("close", (code) => resolve({ code, stdout, stderr }));
  });
}
