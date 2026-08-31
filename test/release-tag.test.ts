import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import assert from "node:assert/strict";
import { test } from "node:test";

const manifest = JSON.parse(readFileSync("package.json", "utf8")) as { version: string };

test("the release guard accepts the exact package tag", () => {
  const result = runGuard(`v${manifest.version}`);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, new RegExp(`release tag: v${escapeRegex(manifest.version)} -> npm next`));
});

test("the release guard rejects a tag for another version", () => {
  const result = runGuard("v999.0.0");

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /does not match/);
});

test("the release guard exports the npm channel for GitHub Actions", () => {
  const directory = mkdtempSync(path.join(tmpdir(), "jecode-release-tag-"));
  const output = path.join(directory, "github-output");

  try {
    const result = runGuard(`v${manifest.version}`, { GITHUB_OUTPUT: output });

    assert.equal(result.status, 0, result.stderr);
    assert.equal(readFileSync(output, "utf8"), "channel=next\n");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

function runGuard(
  tag: string,
  environment: NodeJS.ProcessEnv = {},
): { status: number | null; stdout: string; stderr: string } {
  return spawnSync(process.execPath, ["dev/check-release-tag.ts", tag], {
    cwd: process.cwd(),
    encoding: "utf8",
    env: { ...process.env, ...environment },
    windowsHide: true,
  });
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
