import { test } from "node:test";
import assert from "node:assert/strict";
import { chmod, copyFile, mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { browserCommand } from "../src/external-browser.ts";
import { resolveExecutable } from "../src/executable.ts";

test("executable lookup skips workspace-controlled PATH entries", async () => {
  const area = await mkdtemp(path.join(tmpdir(), "jecode-executable-"));
  const workspace = path.join(area, "workspace");
  const workspaceBin = path.join(workspace, "bin");
  const trustedBin = path.join(area, "trusted");
  const name = process.platform === "win32" ? "rg.exe" : "rg";
  const untrusted = path.join(workspaceBin, name);
  const trusted = path.join(trustedBin, name);

  try {
    await mkdir(workspaceBin, { recursive: true });
    await mkdir(trustedBin, { recursive: true });
    await copyFile(process.execPath, untrusted);
    await copyFile(process.execPath, trusted);
    if (process.platform !== "win32") {
      await chmod(untrusted, 0o755);
      await chmod(trusted, 0o755);
    }

    const searchPath = [workspaceBin, trustedBin].join(path.delimiter);
    assert.equal(
      resolveExecutable("rg", { searchPath, cwd: workspace, rejectUnder: workspace }),
      await realpath(trusted),
    );
    assert.equal(
      resolveExecutable("rg", { searchPath: workspaceBin, cwd: workspace, rejectUnder: workspace }),
      undefined,
    );
  } finally {
    await rm(area, { recursive: true, force: true });
  }
});

test("browser launchers are absent or resolved to an absolute executable", () => {
  const command = browserCommand("https://example.test/");
  if (command !== undefined) assert.equal(path.isAbsolute(command.file), true);
});
