import { test } from "node:test";
import assert from "node:assert/strict";
import {
  MAX_STEERING_CODE_UNITS,
  MAX_STEERING_MESSAGES,
  steeringInbox,
} from "../src/steering.ts";

test("steering drains accepted guidance in order and remains open", () => {
  const states: string[] = [];
  const inbox = steeringInbox((pending, accepting) => states.push(`${pending}:${accepting}`));

  assert.equal(inbox.offer("first"), "queued");
  assert.equal(inbox.offer("second"), "queued");
  assert.deepEqual(inbox.drain(), ["first", "second"]);
  assert.equal(inbox.accepting, true);
  assert.deepEqual(states, ["1:true", "2:true", "0:true"]);
});

test("steering completion closes atomically only when no guidance is pending", () => {
  const inbox = steeringInbox();
  inbox.offer("keep going");

  assert.deepEqual(inbox.drainOrClose(), { messages: ["keep going"], closed: false });
  assert.equal(inbox.offer("one more"), "queued");
  assert.deepEqual(inbox.drain(), ["one more"]);
  assert.deepEqual(inbox.drainOrClose(), { messages: [], closed: true });
  assert.equal(inbox.offer("too late"), "closed");
});

test("steering bounds both queued messages and aggregate text", () => {
  const countBound = steeringInbox();
  for (let index = 0; index < MAX_STEERING_MESSAGES; index++) {
    assert.equal(countBound.offer(String(index)), "queued");
  }
  assert.equal(countBound.offer("overflow"), "full");

  const textBound = steeringInbox();
  assert.equal(textBound.offer("x".repeat(MAX_STEERING_CODE_UNITS)), "queued");
  assert.equal(textBound.offer("x"), "full");
});

test("closing returns guidance that the controller never consumed", () => {
  const states: string[] = [];
  const inbox = steeringInbox((pending, accepting) => states.push(`${pending}:${accepting}`));
  inbox.offer("recover me");

  assert.deepEqual(inbox.close(), ["recover me"]);
  assert.deepEqual(inbox.close(), []);
  assert.equal(inbox.pending, 0);
  assert.equal(inbox.accepting, false);
  assert.deepEqual(states, ["1:true", "0:false"]);
});
