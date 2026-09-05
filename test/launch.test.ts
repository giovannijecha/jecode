import { test } from "node:test";
import assert from "node:assert/strict";
import { parseLaunch } from "../src/launch.ts";

test("parses a fresh launch without changing configuration flags", () => {
  assert.deepEqual(parseLaunch(["--root", "work"]), {
    kind: "new",
    latest: false,
    configArgs: ["--root", "work"],
  });
});

test("plain resume retains the session picker", () => {
  assert.deepEqual(parseLaunch(["resume", "--root", "work"]), {
    kind: "resume",
    latest: false,
    configArgs: ["--root", "work"],
  });
});

test("-c and resume --last share the same launch without changing configuration flags", () => {
  const expected = {
    kind: "resume",
    latest: true,
    configArgs: ["--root", "work"],
  };
  assert.deepEqual(parseLaunch(["-c", "--root", "work"]), expected);
  assert.deepEqual(parseLaunch(["resume", "--last", "--root", "work"]), expected);
  assert.deepEqual(parseLaunch(["resume", "--root", "work", "--last"]), expected);
});

test("rejects unknown launch commands and misplaced or repeated --last", () => {
  assert.throws(() => parseLaunch(["continue"]), /unknown command/);
  assert.throws(() => parseLaunch(["--last"]), /requires `jecode resume`/);
  assert.throws(() => parseLaunch(["resume", "--last", "--last"]), /only once/);
  assert.throws(() => parseLaunch(["-c", "--last"]), /-c already selects the last session/);
});

test("the retired --latest spelling explains both replacements", () => {
  for (const args of [["--latest"], ["resume", "--latest"]]) {
    assert.throws(() => parseLaunch(args), /renamed to --last.*jecode -c.*jecode resume --last/);
  }
});
