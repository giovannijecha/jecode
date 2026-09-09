import assert from "node:assert/strict";
import { test } from "node:test";
import { integratedContextProbe } from "../dev/benchmarks/context-integrated.ts";
import { CORPUS_SEED, mixedCorpus } from "../dev/benchmarks/corpus.ts";

test("mixed corpus is reproducible and exercises source, JSON, prose, logs and Unicode", () => {
  const corpus = mixedCorpus(8_192);
  assert.equal(corpus.length, 8_192);
  assert.equal(corpus, mixedCorpus(8_192));
  assert.notEqual(corpus, mixedCorpus(8_192, CORPUS_SEED + 1));
  for (const pattern of [/export function/, /"enabled":/, /## Decision/, /duration=/, /café 日本 🙂/]) {
    assert.match(corpus, pattern);
  }
  for (const count of [0, -1, 1.5, 5_000_001]) assert.throws(() => mixedCorpus(count));
});

for (const [columns, termination] of [[40, "interrupt"], [120, "disconnect"]] as const) {
  test(`integrated benchmark preserves compacted work across ${termination} and resume at ${columns} columns`,
    { timeout: 45_000 }, async () => {
      const report = await integratedContextProbe(columns, 12, termination);
      assert.equal(report.passed, true);
      assert.equal(report.canonicalResults, 12);
      assert.equal(report.requests, 14);
      assert.equal(report.summaries, 1);
      assert.equal(report.resumedWithoutReplay, true);
      assert.ok(report.measurement.samplesMilliseconds.length > 0);
      assert.ok(report.checkpoint.samplesMilliseconds.length >= 12);
      assert.equal(report.workload.measurement, "production-responses-o200k-reference");
  });
}

test("invalid integrated workload parameters fail before execution", async () => {
  await assert.rejects(integratedContextProbe(0));
  await assert.rejects(integratedContextProbe(60, 0));
});
