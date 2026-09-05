import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { test } from "node:test";
import { assertReleaseDocumentation, releaseChannel } from "../scripts/release-policy.ts";

const guard = fileURLToPath(new URL("../scripts/check-release-tag.ts", import.meta.url));

for (const [version, channel] of [["1.0.0", "latest"], ["1.0.0-rc.1", "next"]] as const) {
  test(`the release guard accepts ${version} and exports its ${channel} channel`, () => {
    const result = runGuard({ name: "@giovannijecha/jecode", version }, [`v${version}`]);

    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, `release tag: v${version} -> npm ${channel}\n`);
    assert.equal(result.githubOutput, `channel=${channel}\n`);
  });

  test(`the release guard rejects a mismatched tag for ${version} without exporting a channel`, () => {
    const result = runGuard({ name: "@giovannijecha/jecode", version }, ["v999.0.0"]);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /does not match/);
    assert.equal(result.githubOutput, undefined);
  });
}

for (const args of [[], ["v1.0.0", "unexpected"]]) {
  test(`the release guard rejects ${args.length === 0 ? "missing" : "extra"} arguments`, () => {
    const result = runGuard({ name: "@giovannijecha/jecode", version: "1.0.0" }, args);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /usage: npm run check:release-tag/);
    assert.equal(result.githubOutput, undefined);
  });
}

test("the release guard rejects a different package name", () => {
  const result = runGuard({ name: "fixture-package", version: "1.0.0" }, ["v1.0.0"]);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /canonical npm package name/);
  assert.equal(result.githubOutput, undefined);
});

test("the release guard rejects a missing package version", () => {
  const result = runGuard({ name: "@giovannijecha/jecode" }, ["v1.0.0"]);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /package version is missing/);
  assert.equal(result.githubOutput, undefined);
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
  manifest: { name: string; version?: string },
  args: string[],
): { status: number | null; stdout: string; stderr: string; githubOutput: string | undefined } {
  const directory = mkdtempSync(path.join(tmpdir(), "jecode-release-tag-"));
  const output = path.join(directory, "github-output");

  try {
    writeFileSync(path.join(directory, "package.json"), JSON.stringify(manifest), "utf8");
    const result = spawnSync(process.execPath, [guard, ...args], {
      cwd: directory,
      encoding: "utf8",
      env: { ...process.env, GITHUB_OUTPUT: output },
      windowsHide: true,
      timeout: 10_000,
    });
    if (result.error !== undefined) throw result.error;
    return {
      status: result.status,
      stdout: result.stdout,
      stderr: result.stderr,
      githubOutput: existsSync(output) ? readFileSync(output, "utf8") : undefined,
    };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
