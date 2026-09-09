import assert from "node:assert/strict";
import { test } from "node:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { integratedContextProbe } from "../dev/benchmarks/context-integrated.ts";
import { CORPUS_SEED, mixedCorpus } from "../dev/benchmarks/corpus.ts";
import { capture, probeEnvironment } from "../dev/benchmarks/capture.ts";

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

test("the integrated probe waits for delayed partial output before interrupting", { timeout: 45_000 }, async () => {
  // Delay the inert server's last text event; announcing a pending response is
  // not evidence that the client has received or painted its partial content.
  const script = `
    import assert from 'node:assert/strict';
    import {registerHooks} from 'node:module';
    registerHooks({load(url, context, next) {
      const loaded = next(url, context);
      if (!url.endsWith('/context-fixture.ts')) return loaded;
      const original = String(loaded.source);
      const statement = 'socketJson(socket, { type: "response.output_text.delta", delta: "Waiting for fixture interruption." });';
      const source = original.replace(statement,
        'setTimeout(() => { if (!socket.destroyed) ' + statement + ' }, 60);');
      assert.notEqual(source, original, 'delayed transport fixture must be installed');
      return {...loaded, source};
    }});
    const {integratedContextProbe} = await import('./dev/benchmarks/context-integrated.ts');
    const report = await integratedContextProbe(40);
    assert.equal(report.canonicalResults, 12);
    assert.equal(report.summaries, 1);
  `;
  const result = await capture(["--input-type=module", "--eval", script],
    fileURLToPath(new URL("../", import.meta.url)),
    probeEnvironment(join(tmpdir(), `jecode-delayed-context-${process.pid}`)), 35_000);
  assert.equal(result.failure, null);
  assert.equal(result.exitCode, 0, result.stderr);
});
