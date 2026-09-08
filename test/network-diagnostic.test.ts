import { test } from "node:test";
import assert from "node:assert/strict";
import { networkErrorCode } from "../src/providers/network-diagnostic.ts";

test("network diagnostics inspect bounded causes and dual-stack connection errors", () => {
  const dns = Object.assign(new Error("private DNS details"), { code: "ENOTFOUND" });
  const tls = Object.assign(new Error("private certificate"), { code: "CERT_HAS_EXPIRED" });
  assert.equal(networkErrorCode(new TypeError("fetch failed", { cause: dns })), "ENOTFOUND");
  assert.equal(networkErrorCode(new AggregateError([new Error("private"), tls])), "CERT_HAS_EXPIRED");
  assert.equal(networkErrorCode(Object.assign(new Error("private"), { code: "PRIVATE_SECRET" })), undefined);
  assert.equal(networkErrorCode({ code: "ECONNRESET" }), undefined);
  assert.equal(networkErrorCode(undefined), undefined);
  const cycle = new Error("private");
  cycle.cause = cycle;
  assert.equal(networkErrorCode(cycle), undefined);
  let nested: Error = dns;
  for (let i = 0; i < 10; i++) nested = new Error("wrapper", { cause: nested });
  assert.equal(networkErrorCode(nested), undefined);
  assert.equal(networkErrorCode(new AggregateError(Array(20).fill(cycle))), undefined);
});
