import { test } from "node:test";
import assert from "node:assert/strict";
import { activityStatus, begin, elapsed } from "../src/tui/activity.ts";

test("an activity owns one abort signal and a stable label", () => {
  const activity = begin("command", "Running /models", 1_000);
  assert.equal(activity.kind, "command");
  assert.equal(activity.label, "Running /models");
  assert.equal(activity.control.signal.aborted, false);
  activity.control.abort(new Error("stop"));
  assert.equal(activity.control.signal.aborted, true);
});

test("elapsed time is compact and cannot go negative", () => {
  const activity = begin("turn", "Waiting", 10_000);
  assert.equal(elapsed(activity, 9_000), "0s");
  assert.equal(elapsed(activity, 69_999), "59s");
  assert.equal(elapsed(activity, 75_000), "1m 05s");
  assert.equal(activityStatus(activity, "Writing", 75_000), "Writing · 1m 05s");
});
