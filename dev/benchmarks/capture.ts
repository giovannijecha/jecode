// Bounded child-process capture; no shell, inherited Node flags, or provider secrets.

import { spawn } from "node:child_process";

export interface Capture {
  exitCode: number | null;
  signal: string | null;
  failure: string | null;
  stdout: string;
  stderr: string;
}

export function probeEnvironment(home: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { NO_COLOR: "1", JECODE_HOME: home };
  for (const name of ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"]) {
    const key = Object.keys(process.env).find((key) => key.toLowerCase() === name.toLowerCase());
    if (key !== undefined) env[name] = process.env[key];
  }
  return env;
}

export function capture(
  args: string[], cwd: string, env: NodeJS.ProcessEnv,
  timeoutMs = 180_000, maxBytes = 1_048_576, signal?: AbortSignal,
): Promise<Capture> {
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve({ exitCode: null, signal: null, failure: "cancelled", stdout: "", stderr: "" });
      return;
    }
    const child = spawn(process.execPath, args, { cwd, env, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let bytes = 0;
    let failure: string | null = null;
    const stop = (reason: string): void => {
      failure ??= reason;
      child.kill("SIGKILL");
    };
    const cancel = (): void => stop("cancelled");
    const timer = setTimeout(() => stop("timeout"), timeoutMs);
    signal?.addEventListener("abort", cancel, { once: true });
    child.stdout.on("data", (chunk: Buffer) => {
      bytes += chunk.length;
      if (bytes <= maxBytes) stdout.push(chunk);
      else stop("output limit exceeded");
    });
    child.stderr.on("data", (chunk: Buffer) => {
      bytes += chunk.length;
      if (bytes <= maxBytes) stderr.push(chunk);
      else stop("output limit exceeded");
    });
    child.on("error", () => { failure ??= "could not start probe"; });
    child.on("close", (exitCode, exitSignal) => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", cancel);
      resolve({ exitCode, signal: exitSignal, failure,
        stdout: Buffer.concat(stdout).toString("utf8"), stderr: Buffer.concat(stderr).toString("utf8") });
    });
  });
}
