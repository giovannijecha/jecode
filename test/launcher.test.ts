import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { MAX_PROMPT_CODE_UNITS } from "../src/input-boundary.ts";

function runExecutable(args: string[]): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return runNode(path.resolve("bin/jecode.js"), args);
}

function runLauncherAsVersion(
  version: string,
): Promise<{ code: number | null; stdout: string; stderr: string }> {
  const launcher = new URL("../bin/jecode.js", import.meta.url).href;
  const source = [
    `Object.defineProperty(process.versions, "node", { value: ${JSON.stringify(version)} });`,
    `await import(${JSON.stringify(launcher)});`,
  ].join("\n");
  return runNode("--input-type=module", ["--eval", source]);
}

function runNode(
  executable: string,
  args: string[],
  options: { input?: string; environment?: NodeJS.ProcessEnv } = {},
): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const spawnOptions = { cwd: process.cwd(), env: options.environment };
    const child =
      options.input === undefined
        ? spawn(process.execPath, [executable, ...args], {
            ...spawnOptions,
            stdio: ["ignore", "pipe", "pipe"],
          })
        : spawn(process.execPath, [executable, ...args], {
            ...spawnOptions,
            stdio: ["pipe", "pipe", "pipe"],
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
    if (options.input !== undefined) child.stdin?.end(options.input);
    child.once("error", reject);
    child.once("close", (code) => resolve({ code, stdout, stderr }));
  });
}

test("the packaged jecode executable reaches batch help without a developer script", async () => {
  const result = await runExecutable(["--help"]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /Usage:\s+jecode \[options\]/);
  assert.match(result.stdout, /--provider/);
  assert.match(result.stdout, /optional per-turn model-request budget/);
  assert.match(result.stdout, /type \/ to discover/);
  assert.equal(result.stderr, "");
});

test("the packaged batch executable reports terminal failures on stderr", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-batch-home-"));
  const environment: NodeJS.ProcessEnv = {
    ...process.env,
    HOME: directory,
    USERPROFILE: directory,
    APPDATA: directory,
    LOCALAPPDATA: directory,
    JECODE_HOME: directory,
    XDG_CONFIG_HOME: directory,
  };
  for (const name of Object.keys(environment)) {
    if (name.toUpperCase() === "ANTHROPIC_API_KEY") delete environment[name];
  }

  try {
    const result = await runNode(
      path.resolve("bin/jecode.js"),
      ["--provider", "anthropic", "--model", "fixture-model"],
      { input: "hello\n", environment },
    );

    assert.equal(result.code, 1);
    assert.match(result.stdout, /> hello/);
    assert.doesNotMatch(result.stdout, /ANTHROPIC_API_KEY is not set/);
    assert.match(result.stderr, /^jecode: Anthropic API: ANTHROPIC_API_KEY is not set/m);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("the packaged batch executable rejects an oversized final line on stderr", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-batch-limit-home-"));
  const environment: NodeJS.ProcessEnv = {
    ...process.env,
    HOME: directory,
    USERPROFILE: directory,
    APPDATA: directory,
    LOCALAPPDATA: directory,
    JECODE_HOME: directory,
    XDG_CONFIG_HOME: directory,
  };

  try {
    const result = await runNode(
      path.resolve("bin/jecode.js"),
      ["--provider", "anthropic", "--model", "fixture-model", "--ephemeral"],
      { input: "x".repeat(MAX_PROMPT_CODE_UNITS + 1), environment },
    );

    assert.equal(result.code, 1);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /^jecode: Prompt cannot exceed 1,048,576 UTF-16 code units/m);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("the packaged executable reports its manifest version", async () => {
  const manifest = JSON.parse(await readFile("package.json", "utf8")) as { version?: unknown };
  const result = await runExecutable(["--version"]);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout.trim(), manifest.version);
});

test("the launcher rejects the unverified Node 23 type-stripping gap", async () => {
  const result = await runLauncherAsVersion("23.5.0");
  assert.equal(result.code, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /requires Node\.js 22\.18\+ \(22\.x\) or Node\.js 24\+/);
  assert.match(result.stderr, /current: 23\.5\.0/);
});

test("a source-only executable explains how to obtain its missing runtime", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-missing-runtime-"));
  const bin = path.join(directory, "bin");

  try {
    await mkdir(bin);
    await copyFile("package.json", path.join(directory, "package.json"));
    await copyFile("bin/jecode.js", path.join(bin, "jecode.js"));

    const result = await runNode(path.join(bin, "jecode.js"), []);
    assert.equal(result.code, 1);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /install @giovannijecha\/jecode from npm/);
    assert.match(result.stderr, /npm run build:release/);
    assert.doesNotMatch(result.stderr, /ERR_MODULE_NOT_FOUND/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
