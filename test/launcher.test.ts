import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";

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

test("the packaged jecode executable shows help without a terminal or developer script", async () => {
  const result = await runExecutable(["--help"]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /Usage:\s+jecode \[options\]/);
  assert.match(result.stdout, /jecode -c \[options\]/);
  assert.match(result.stdout, /jecode resume \[--last\] \[options\]/);
  assert.doesNotMatch(result.stdout, /--latest/);
  assert.doesNotMatch(result.stdout, /--provider|--model|--effort|--max-tokens|--max-steps|--compaction-percent|--auto-approve/);
  assert.match(result.stdout, /interactive terminal on stdin and stdout/);
  assert.match(result.stdout, /Type \/ to discover/);
  assert.equal(result.stderr, "");
});

test("the packaged executable redirects the retired resume spelling before startup", async () => {
  const result = await runExecutable(["resume", "--latest"]);
  assert.equal(result.code, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /--latest has been renamed to --last.*jecode -c.*jecode resume --last/);
});

test("the packaged executable rejects non-interactive launches before configuration or input", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-terminal-home-"));
  const environment: NodeJS.ProcessEnv = {
    ...process.env,
    JECODE_HOME: directory,
    JECODE_PROVIDER: "retired-provider-override",
    OLLAMA_HOST: "http://127.0.0.1:11434",
  };
  try {
    for (const args of [[], ["-c"], ["resume", "--last"]]) {
      for (const input of [undefined, "do not process this request\n"]) {
        const result = await runNode(path.resolve("bin/jecode.js"), args, {
          environment, ...(input === undefined ? {} : { input }),
        });
        assert.equal(result.code, 1);
        assert.equal(result.stdout, "");
        assert.match(result.stderr, /^jecode: an interactive terminal is required on stdin and stdout/m);
        assert.doesNotMatch(result.stderr, /retired-provider|127|do not process/);
        assert.deepEqual(await readdir(directory), []);
      }
    }
    for (const args of [["--help"], ["--version"]]) {
      const result = await runNode(path.resolve("bin/jecode.js"), args, { environment });
      assert.equal(result.code, 0, result.stderr);
      assert.notEqual(result.stdout, "");
      assert.equal(result.stderr, "");
    }
    assert.deepEqual(await readdir(directory), []);
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

test("either redirected stream prevents terminal takeover", async () => {
  const launcher = new URL("../bin/jecode.js", import.meta.url).href;
  for (const terminalStream of ["stdin", "stdout"]) {
    const source = [
      `Object.defineProperty(process.${terminalStream}, "isTTY", { value: true });`,
      `await import(${JSON.stringify(launcher)});`,
    ].join("\n");
    const result = await runNode("--input-type=module", ["--eval", source]);
    assert.equal(result.code, 1);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /interactive terminal is required on stdin and stdout/);
  }
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
