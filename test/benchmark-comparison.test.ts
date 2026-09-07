import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { compare, markdown } from "../dev/benchmarks/comparison.ts";
import { collectionComplete, sample, validateCollection } from "../dev/benchmarks/collection.ts";
import type { Collection } from "../dev/benchmarks/collection.ts";
import { capture, probeEnvironment } from "../dev/benchmarks/capture.ts";
import { probes } from "../dev/benchmarks/probes.ts";

function fixture(): Collection {
  const results = [
    { iterations: 5, tokenizer: { inputCharacters: 100 }, request: { inputCharacters: 200 },
      planning: { messages: 12, inputCharacters: 300 }, workflows: [{ reads: 12, outputCharactersPerRead: 40 }] },
    { secrets: 100, outputCharacters: 300, iterations: 5 },
    { files: 600, fileBytes: 4096, inputBytes: 2457600, iterations: 5 },
    { checkpoint: { iterations: 5, results: [{ nodes: 50 }] }, catalog: { iterations: 5,
      sessions: 12, results: [{ nodesPerSession: 200 }] }, load: { iterations: 3, results: [{ nodes: 1024 }] } },
    { blocks: 20001, liveReasoningCharacters: 200000, viewports: [{ columns: 100 }],
      cachedResize: { frames: 100 }, stable: { frames: 500 }, streaming: { frames: 200 }, expandedLive: { frames: 200 } },
    { color: false, reducedMotion: true, forcedGcBetweenPhases: true, measurement: "fixture",
      results: [{ columns: 120, rows: 40, historyBlocks: 2000, warmup: 25, outputBytesPerSecond: null,
        scenarios: { typing: { samples: 100 } }, memory: { afterClose: { rss: 1000 } } }] },
  ];
  return { schema: 1, commit: "a".repeat(40), dirty: false, capturedAt: "2026-09-07T00:00:00.000Z",
    environment: { node: "v24.18.0", platform: "linux", arch: "x64", osRelease: "fixture",
      cpu: "fixture CPU", cpuCount: 4, totalMemoryBytes: 1000000, color: "off", runnerImage: "", runnerImageVersion: "" },
    repetitions: 3, probes: probes.map((probe, index) => ({ name: probe.name, sourceHash: "b".repeat(64),
      samples: [10, 11, 100].map((time) => ({ exitCode: 0, failure: null, stderr: "",
        results: { ...structuredClone(results[index]), medianMilliseconds: time, thresholds: { medianMilliseconds: 20 } } })) })) };
}

test("compares medians and ranges, excludes thresholds, and retains raw evidence", () => {
  const a = fixture();
  const b = fixture();
  b.commit = "c".repeat(40);
  b.probes[0]!.samples.forEach((sample, index) => { sample.results!["medianMilliseconds"] = [12, 13, 14][index]; });
  const result = compare(validateCollection(a), validateCollection(b));
  assert.ok(result.probes.every((probe) => probe.status === "compared"));
  const metric = result.probes[0]!.metrics[0]!;
  assert.equal(metric.path, "medianMilliseconds");
  assert.deepEqual(metric.baseline, { median: 11, min: 10, max: 100 });
  assert.equal(metric.current.median, 13);
  assert.equal(metric.rangesOverlap, true);
  assert.ok(Math.abs(metric.changePercent! - 18.1818) < 0.001);
  assert.equal(result.probes[5]!.metrics.find((metric) => metric.path.endsWith("rss"))!.unit, "bytes");
  assert.match(markdown(result), /not regression verdicts/);
  assert.equal(a.probes[0]!.samples[2]!.results!["medianMilliseconds"], 100);
});

test("rejects different environments, dirty runs, methods, repetition counts, and workload", () => {
  for (const change of [
    (b: Collection) => { b.environment["node"] = "v24.19.0"; },
    (b: Collection) => { b.environment["cpu"] = "different CPU"; },
    (b: Collection) => { b.dirty = true; },
    (b: Collection) => { b.probes[0]!.sourceHash = "d".repeat(64); },
    (b: Collection) => { b.repetitions = 2; },
    (b: Collection) => { b.probes[0]!.samples[1]!.results!["iterations"] = 9; },
    (b: Collection) => { delete b.probes[0]!.samples[1]!.results!["iterations"]; },
  ]) {
    const b = fixture();
    change(b);
    assert.equal(compare(fixture(), b).probes[0]!.status, "incompatible");
  }
});

test("failed and interrupted probes never produce improvement ratios", () => {
  for (const change of [
    (b: Collection) => { b.probes[0]!.samples[0]!.failure = "timeout"; },
    (b: Collection) => { b.probes[0]!.samples[0]!.exitCode = 1; },
    (b: Collection) => { b.probes[0]!.samples[0]!.results = null; },
    (b: Collection) => { b.probes[0]!.samples[0]!.results!["passed"] = false; },
  ]) {
    const b = fixture(); change(b);
    const result = compare(fixture(), b);
    assert.equal(result.probes[0]!.status, "failed");
    assert.equal(result.probes[0]!.metrics.length, 0);
  }
});

test("zero baselines and missing numeric samples do not become invented percentages", () => {
  const a = fixture(); const b = fixture();
  for (const sample of a.probes[0]!.samples) sample.results!["medianMilliseconds"] = 0;
  assert.equal(compare(a, b).probes[0]!.metrics[0]!.changePercent, null);
  b.probes[5]!.samples[0]!.results!["medianMilliseconds"] = null;
  const tui = compare(a, b).probes[5]!;
  assert.ok(tui.unavailable.includes("medianMilliseconds"));
  assert.ok(!tui.metrics.some((metric) => metric.path === "medianMilliseconds"));
  b.probes[0]!.samples[0]!.results!["medianMilliseconds"] = Infinity;
  assert.equal(compare(a, b).probes[0]!.status, "incompatible");
});

test("validates complete collections and rejects duplicate probes and missing metadata", () => {
  assert.throws(() => validateCollection({}));
  for (const change of [
    (b: Collection) => { b.probes.pop(); },
    (b: Collection) => { b.probes[1] = b.probes[0]!; },
    (b: Collection) => { b.probes[0]!.samples.pop(); },
    (b: Collection) => { b.commit = "branch-name"; },
    (b: Collection) => { delete b.environment["cpu"]; },
  ]) { const b = fixture(); change(b); assert.throws(() => validateCollection(b)); }
});

test("probe report validation preserves a failed report and rejects wrong sources", () => {
  const probe = probes[2];
  const results = fixture().probes[2]!.samples[0]!.results!;
  const report = { benchmark: probe.benchmark, environment: {
    node: process.version, platform: process.platform, arch: process.arch }, results };
  const captured = { exitCode: 1, signal: null, failure: null, stdout: JSON.stringify(report), stderr: "fixture failure" };
  assert.deepEqual(sample(captured, probe).results, results);
  assert.equal(sample(captured, probe).failure, "unexpected probe exit");
  const negative = { ...report, results: { ...results, passed: false } };
  assert.equal(sample({ ...captured, stdout: JSON.stringify(negative) }, probe).failure, "probe reported failure");
  assert.equal(sample({ ...captured, stdout: "{}" }, probe).failure, "missing or invalid probe report");
  assert.equal(sample({ ...captured, failure: "timeout" }, probe).results, null);
});

test("a measured failure stays visible without treating successful acquisition as a timing gate", () => {
  const a = fixture();
  const negative = a.probes[3]!.samples[0]!;
  negative.results!["passed"] = false;
  negative.exitCode = 1;
  negative.failure = "probe reported failure";
  assert.equal(collectionComplete(validateCollection(a)), true);
  assert.equal(compare(fixture(), a).probes[3]!.status, "failed");
  negative.failure = "timeout";
  assert.equal(collectionComplete(a), false);
  negative.failure = "unexpected probe exit";
  assert.equal(collectionComplete(a), false);
  negative.failure = "probe reported failure";
  negative.exitCode = 2;
  assert.equal(collectionComplete(a), false);
});

test("public comparison command writes reports and rejects malformed and oversized input", async () => {
  const root = await mkdtemp(join(tmpdir(), "jecode-benchmark-test-"));
  try {
    const base = join(root, "base.json"); const current = join(root, "current.json");
    await writeFile(base, JSON.stringify(fixture())); await writeFile(current, JSON.stringify(fixture()));
    const args = ["dev/benchmarks/compare.ts", base, current, root];
    const run = () => capture(args, resolve("."), probeEnvironment(join(root, "home")), 10000);
    assert.equal((await run()).exitCode, 0);
    assert.equal(JSON.parse(await readFile(join(root, "comparison.json"), "utf8")).probes.length, 6);
    assert.match(await readFile(join(root, "SUMMARY.md"), "utf8"), /Benchmark comparison/);
    const negative = fixture();
    negative.probes[3]!.samples[0] = { exitCode: 1, failure: "probe reported failure", stderr: "",
      results: { ...negative.probes[3]!.samples[0]!.results, passed: false } };
    await writeFile(current, JSON.stringify(negative));
    assert.equal((await run()).exitCode, 0);
    assert.match(await readFile(join(root, "SUMMARY.md"), "utf8"), /session \| failed/);
    negative.probes[3]!.samples[0]!.failure = "timeout";
    await writeFile(current, JSON.stringify(negative));
    assert.equal((await run()).exitCode, 1);
    await writeFile(current, "{"); assert.notEqual((await run()).exitCode, 0);
    await writeFile(current, " ".repeat(8 * 1048576 + 1));
    assert.match((await run()).stderr, /exceeds 8 MiB/);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("capture bounds output, terminates hangs, handles cancellation, and filters secrets", async () => {
  const env = probeEnvironment(join(tmpdir(), "absent-benchmark-home"));
  assert.equal(env["NODE_OPTIONS"], undefined);
  assert.equal(env["OPENAI_API_KEY"], undefined);
  assert.equal(env["GITHUB_TOKEN"], undefined);
  assert.equal(env["NO_COLOR"], "1");
  const output = await capture(["-e", "process.stdout.write('x'.repeat(10000));setInterval(()=>{},1000)"], resolve("."), env, 10000, 1024);
  assert.equal(output.failure, "output limit exceeded");
  assert.ok(output.stdout.length <= 1024);
  const timeout = await capture(["-e", "setInterval(()=>{},1000)"], resolve("."), env, 150);
  assert.equal(timeout.failure, "timeout");
  const abort = new AbortController();
  const pending = capture(["-e", "setInterval(()=>{},1000)"], resolve("."), env, 10000, 1024, abort.signal);
  abort.abort(); assert.equal((await pending).failure, "cancelled");
  assert.equal((await capture([], resolve("."), env, 10000, 1024, abort.signal)).failure, "cancelled");
  assert.equal((await capture(["-e", "0"], join(tmpdir(), "missing-benchmark-cwd", "nested"), env)).failure, "could not start probe");
});
