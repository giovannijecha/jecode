// Shell tool. One command, captured output, a timeout, and a hard cap on how
// much of it comes back.

import { spawn } from "node:child_process";
import * as path from "node:path";
import type { Tool } from "./types.ts";
import { optionalInt, requireString } from "./args.ts";
import { credentialRedactor, redactCredentials, shellEnvironment } from "../credential-safety.ts";
import { resolveExecutable } from "../executable.ts";

const DEFAULT_TIMEOUT_MS = 120_000;
const MAX_TIMEOUT_MS = 2_147_483_647;
const MAX_OUTPUT_CHARS = 30_000;
const PIPE_DRAIN_MS = 100;

export const runCommand: Tool = {
  name: "run_command",
  description:
    "Run a shell command starting in the workspace root and return its combined stdout " +
    "and stderr. The shell is not a filesystem sandbox, so calls ask for approval by default. Output is " +
    "truncated past 30000 characters.",
  dangerous: true,
  concurrency: "exclusive",
  input: {
    type: "object",
    properties: {
      command: { type: "string", description: "The command line to run." },
      timeout_ms: { type: "integer", description: "Kill the command after this long. Defaults to 120000." },
    },
    required: ["command"],
  },
  async run(args, ctx) {
    const command = requireString(args, "command");
    const timeoutMs = optionalInt(args, "timeout_ms") ?? DEFAULT_TIMEOUT_MS;
    if (timeoutMs <= 0) throw new Error('"timeout_ms" must be a positive integer');
    if (timeoutMs > MAX_TIMEOUT_MS) {
      throw new Error(`"timeout_ms" must be at most ${MAX_TIMEOUT_MS}ms`);
    }
    const result = await execute(command, ctx.root, timeoutMs, ctx.signal, ctx.onOutput);

    const output = redactCredentials(result.output);
    const summary = result.timedOut
      ? `timed out after ${timeoutMs}ms`
      : `exit ${result.code ?? "?"}`;

    return {
      output: output === "" ? `[${summary}]` : `${output}\n[${summary}]`,
      summary,
      isError: result.timedOut || result.code !== 0,
    };
  },
};

type Outcome = { output: string; code: number | null; timedOut: boolean };

function execute(
  command: string,
  cwd: string,
  timeoutMs: number,
  signal: AbortSignal | undefined,
  onOutput: ToolContextOutput,
): Promise<Outcome> {
  if (signal?.aborted === true) return Promise.reject(abortReason(signal));

  return new Promise((resolve, reject) => {
    const child = spawn(command, {
      cwd,
      env: shellEnvironment(),
      shell: true,
      windowsHide: true,
      detached: process.platform !== "win32",
      stdio: ["ignore", "pipe", "pipe"],
    });
    const output = capture(onOutput);
    let timedOut = false;
    let aborted: Error | undefined;
    let settled = false;
    let forceTimer: NodeJS.Timeout | undefined;
    let drainTimer: NodeJS.Timeout | undefined;

    const timer = setTimeout(() => {
      timedOut = true;
      requestStop();
    }, timeoutMs);

    const requestStop = () => {
      stopTree(child.pid, false);
      forceTimer ??= setTimeout(() => stopTree(child.pid, true), 500);
    };

    const onAbort = () => {
      aborted = signal === undefined ? new Error("aborted") : abortReason(signal);
      requestStop();
    };
    signal?.addEventListener("abort", onAbort, { once: true });

    const cleanup = () => {
      clearTimeout(timer);
      if (forceTimer !== undefined) clearTimeout(forceTimer);
      if (drainTimer !== undefined) clearTimeout(drainTimer);
      signal?.removeEventListener("abort", onAbort);
    };

    const finish = (code: number | null): void => {
      if (settled) return;
      settled = true;
      cleanup();
      if (aborted !== undefined) reject(aborted);
      else resolve({ output: output.value().trimEnd(), code, timedOut });
    };

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", output.append);
    child.stderr.on("data", output.append);

    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error);
    });

    child.on("exit", (code) => {
      // The group can outlive its leader. Once a requested stop makes the
      // shell exit, force the remaining descendants before cleanup cancels the
      // fallback timer.
      if (timedOut || aborted !== undefined) stopTree(child.pid, true);
      // `close` normally follows once both pipes drain. A detached descendant
      // can inherit those descriptors after the command itself has exited,
      // though, so bound that final drain instead of hanging the tool on it.
      drainTimer = setTimeout(() => {
        child.stdout.destroy();
        child.stderr.destroy();
        finish(code);
      }, PIPE_DRAIN_MS);
    });
    child.on("close", finish);
  });
}

type ToolContextOutput = ((output: string) => void) | undefined;

function capture(onOutput?: ToolContextOutput): { append(chunk: string): void; value(): string } {
  const half = MAX_OUTPUT_CHARS / 2;
  const credentials = credentialRedactor();
  let head = "";
  let tail = "";
  let total = 0;

  const appendSafe = (chunk: string): void => {
    total += chunk.length;
    const room = Math.max(0, half - head.length);
    head += chunk.slice(0, room);
    const rest = chunk.slice(room);
    if (rest !== "") tail = `${tail}${rest}`.slice(-half);
    if (chunk !== "") onOutput?.(formatted().trimEnd());
  };

  const formatted = (): string => {
    if (total <= MAX_OUTPUT_CHARS) return `${head}${tail}`;
    const cut = total - head.length - tail.length;
    return `${head}\n\n[... ${cut} characters cut ...]\n\n${tail}`;
  };

  return {
    append(chunk) {
      appendSafe(credentials.write(chunk));
    },
    value() {
      appendSafe(credentials.end());
      return formatted();
    },
  };
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error("aborted");
}

function stopTree(pid: number | undefined, force: boolean): void {
  if (pid === undefined) return;

  if (process.platform === "win32") {
    const args = ["/pid", String(pid), "/T", ...(force ? ["/F"] : [])];
    const windows = process.env["SystemRoot"] ?? process.env["WINDIR"];
    const taskkill = windows === undefined
      ? undefined
      : resolveExecutable("taskkill.exe", {
          searchPath: path.join(windows, "System32"),
          rejectUnder: process.cwd(),
        });
    if (taskkill !== undefined) {
      const killer = spawn(taskkill, args, {
        cwd: path.dirname(taskkill),
        windowsHide: true,
        stdio: "ignore",
      });
      killer.on("error", () => undefined);
      return;
    }
    try {
      process.kill(pid, force ? "SIGKILL" : "SIGTERM");
    } catch {
      // The process may already be gone, or the platform helper unavailable.
    }
    return;
  }

  try {
    process.kill(-pid, force ? "SIGKILL" : "SIGTERM");
  } catch {
    // The process may have exited between the timeout and this signal.
  }
}
