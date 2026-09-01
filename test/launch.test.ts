import { test } from "node:test";
import assert from "node:assert/strict";
import { parseLaunch } from "../src/launch.ts";

test("parses a fresh launch without changing configuration flags", () => {
  assert.deepEqual(parseLaunch(["--model", "gpt-5"]), {
    kind: "new",
    latest: false,
    configArgs: ["--model", "gpt-5"],
  });
});

test("parses resume and owns --latest outside runtime configuration", () => {
  assert.deepEqual(parseLaunch(["resume", "--latest", "--root", "work"]), {
    kind: "resume",
    latest: true,
    configArgs: ["--root", "work"],
  });
});

test("rejects unknown launch commands and misplaced --latest", () => {
  assert.throws(() => parseLaunch(["continue"]), /unknown command/);
  assert.throws(() => parseLaunch(["--latest"]), /requires `jecode resume`/);
  assert.throws(() => parseLaunch(["resume", "--latest", "--latest"]), /only once/);
});
