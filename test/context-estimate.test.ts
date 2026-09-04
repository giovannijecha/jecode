import { test } from "node:test";
import assert from "node:assert/strict";
import { constants, deflateRawSync } from "node:zlib";
import {
  estimateSerializedTokens,
  estimateSerializedTokensResponsive,
} from "../src/context/estimate.ts";
import { estimateTokensResponsive } from "../src/context/policy.ts";
import type { Message } from "../src/types.ts";

test("chunked estimates equal the responsive path and remain conservative", async () => {
  const values = [
    "ordinary source text ".repeat(10_000),
    String.fromCodePoint(0x10ffff).repeat(40_000),
    entropy(200_000),
  ];

  for (const value of values) {
    const estimated = estimateSerializedTokens(value);
    assert.equal(await estimateSerializedTokensResponsive(value), estimated);
    assert.ok(estimated >= monolithicEstimate(value));
  }
});

test("an 8 MiB estimate yields to timers before the test deadline", {
  timeout: 5_000,
}, async () => {
  const value = "x".repeat(8 * 1_024 * 1_024);
  let ticks = 0;
  const timer = setInterval(() => ticks++, 1);
  try {
    assert.ok(await estimateSerializedTokensResponsive(value) > 0);
  } finally {
    clearInterval(timer);
  }

  assert.ok(ticks > 1, `estimation blocked the event loop (timer ticks: ${ticks})`);
});

test("responsive estimation observes cancellation between chunks", async () => {
  const control = new AbortController();
  setImmediate(() => control.abort(new Error("cancelled estimate")));

  await assert.rejects(
    estimateSerializedTokensResponsive("x".repeat(8 * 1_024 * 1_024), control.signal),
    /cancelled estimate/,
  );
});

test("a changed projection is estimated from its new content", async () => {
  const projection: Message[] = [{
    role: "user",
    content: [{ kind: "text", text: "short" }],
  }];
  const before = await estimateTokensResponsive(projection);
  projection[0] = {
    role: "user",
    content: [{ kind: "text", text: entropy(20_000) }],
  };

  assert.ok(await estimateTokensResponsive(projection) > before);
});

function monolithicEstimate(value: unknown): number {
  const serialized = Buffer.from(JSON.stringify(value), "utf8");
  const compressedBytes = deflateRawSync(serialized, {
    level: constants.Z_BEST_SPEED,
  }).byteLength;
  let compactableAscii = 0;
  let literalBytes = 0;
  for (const byte of serialized) {
    if (
      (byte >= 48 && byte <= 57) ||
      (byte >= 65 && byte <= 90) ||
      (byte >= 97 && byte <= 122)
    ) compactableAscii++;
    else literalBytes++;
  }
  return Math.min(serialized.byteLength, Math.max(
    Math.ceil(serialized.byteLength / 3),
    Math.ceil(compressedBytes / (5 / 6)),
    literalBytes + Math.ceil(compactableAscii / 3),
  ));
}

function entropy(length: number): string {
  let seed = 0x12345678;
  return Array.from({ length }, () => {
    seed = (seed * 1_664_525 + 1_013_904_223) >>> 0;
    return String.fromCharCode(33 + seed % 94);
  }).join("");
}
