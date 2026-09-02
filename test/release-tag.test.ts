import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import assert from "node:assert/strict";
import { test } from "node:test";
import { assertReleaseDocumentation, releaseChannel } from "../dev/release-policy.ts";

const manifest = JSON.parse(readFileSync("package.json", "utf8")) as { version: string };

test("the release guard accepts the exact package tag", () => {
  const result = runGuard(`v${manifest.version}`);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, new RegExp(`release tag: v${escapeRegex(manifest.version)} -> npm latest`));
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
    assert.equal(readFileSync(output, "utf8"), "channel=latest\n");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("release channels ignore build metadata when identifying a prerelease", () => {
  assert.equal(releaseChannel("1.0.0"), "latest");
  assert.equal(releaseChannel("1.0.0+build-test"), "latest");
  assert.equal(releaseChannel("1.0.0-rc.1+build-test"), "next");
});

test("stable documentation cannot advertise an inactive next tag", () => {
  const command = "npm install --global @giovannijecha/jecode@next";

  assert.throws(() => assertReleaseDocumentation("1.0.0", command), /inactive npm next tag/);
  assert.doesNotThrow(() => assertReleaseDocumentation("1.0.0-rc.1", command));
  assert.doesNotThrow(() => assertReleaseDocumentation("1.0.0", "install the stable release"));
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
