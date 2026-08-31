// Open one HTTPS URL without involving a shell or interpolating commands.

import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";

export async function openExternal(url: string): Promise<boolean> {
  const target = new URL(url);
  if (target.protocol !== "https:") throw new Error("only HTTPS links may be opened");

  const command = browserCommand(target.href);
  if (command === undefined) return false;

  return new Promise((resolve) => {
    const child = spawn(command.file, command.args, {
      detached: true,
      stdio: "ignore",
      windowsHide: true,
    });
    child.once("error", () => resolve(false));
    child.once("spawn", () => {
      child.unref();
      resolve(true);
    });
  });
}

function browserCommand(url: string): { file: string; args: string[] } | undefined {
  if (process.platform === "win32") {
    return { file: "rundll32.exe", args: ["url.dll,FileProtocolHandler", url] };
  }
  if (process.platform === "darwin") return { file: "open", args: [url] };
  if (isWsl()) return { file: "explorer.exe", args: [url] };
  return { file: "xdg-open", args: [url] };
}

export function headlessEnvironment(): boolean {
  if (isWsl() || process.env["SSH_CONNECTION"] !== undefined || process.env["SSH_TTY"] !== undefined) {
    return true;
  }
  return process.platform === "linux" &&
    process.env["DISPLAY"] === undefined &&
    process.env["WAYLAND_DISPLAY"] === undefined;
}

function isWsl(): boolean {
  if (process.platform !== "linux") return false;
  if (process.env["WSL_DISTRO_NAME"] !== undefined || process.env["WSL_INTEROP"] !== undefined) return true;
  try {
    return /microsoft/i.test(readFileSync("/proc/version", "utf8"));
  } catch {
    return false;
  }
}
