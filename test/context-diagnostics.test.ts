import { test } from "node:test";
import assert from "node:assert/strict";
import { channel } from "node:diagnostics_channel";
import { mkdir, mkdtemp, open, readFile, readdir, rm, stat, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { CONTEXT_DIAGNOSTIC_CHANNEL, safeDiagnostic } from "../src/context/diagnostics.ts";
import type { ContextDiagnostic } from "../src/context/diagnostics.ts";
import { compactContext } from "../src/context/compactor.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { provider } from "../dev/test-support/app.ts";
import { contextRecorder } from "../dev/context/recorder.ts";

const request = { kind: "request", source: "provider-prefix", estimatedTokens: 130_000,
  inputTokens: 58_300, reportedInputTokens: 58_040 } as const;

test("diagnostic records whitelist fields and reject malformed counters", () => {
  assert.deepEqual(safeDiagnostic({ ...request, secret: "private", messages: ["private"] }), request);
  for (const value of [null, "private", {}, { ...request, inputTokens: NaN },
    { ...request, reportedInputTokens: -1 }, { ...request, source: "private" }]) {
    assert.equal(safeDiagnostic(value), undefined);
  }
});

test("recorder bounds output, strips private fields, closes once, and stops listening", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-context-record-"));
  const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  try {
    const recorder = await contextRecorder(root, 3);
    source.publish({ ...request, credential: "synthetic-private-value" });
    for (let i = 0; i < 9; i++) source.publish(request);
    const summary = await recorder.close();
    assert.deepEqual(summary, { written: 3, dropped: 7, writeFailed: false });
    assert.deepEqual(await recorder.close(), summary);
    const text = await readFile(recorder.file, "utf8");
    assert.doesNotMatch(text, /credential|synthetic-private/);
    const records = text.trim().split("\n").map((line) => JSON.parse(line));
    assert.equal(records.length, 4);
    assert.deepEqual(records.slice(0, 3).map((r) => r.sequence), [1, 2, 3]);
    assert.equal(records[3].kind, "end");
    source.publish(request);
    assert.equal(await readFile(recorder.file, "utf8"), text);
    if (process.platform !== "win32") assert.equal((await stat(recorder.file)).mode & 0o777, 0o600);
    await assert.rejects(contextRecorder(root, 0), /invalid context recording limit/);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("recording failures do not interrupt the controller and are reported on close", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-context-record-failure-"));
  try {
    const probe = await open(path.join(root, "probe"), "wx");
    const prototype = Object.getPrototypeOf(probe);
    await probe.close();
    const recorder = await contextRecorder(root);
    const fault = t.mock.method(prototype, "writeFile", async () => { throw new Error("synthetic I/O failure"); });
    channel(CONTEXT_DIAGNOSTIC_CHANNEL).publish(request);
    const result = await recorder.close();
    fault.mock.restore();
    assert.deepEqual(result, { written: 0, dropped: 1, writeFailed: true });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("recording rejects a linked output directory", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-context-record-link-"));
  try {
    const target = path.join(root, "target");
    const linked = path.join(root, "linked");
    await mkdir(target);
    await symlink(target, linked, process.platform === "win32" ? "junction" : "dir");
    await assert.rejects(contextRecorder(linked), /not a direct directory/);
    assert.deepEqual(await readdir(target), []);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("cancelled and failed manual summaries publish content-free outcomes", async () => {
  const events: ContextDiagnostic[] = [];
  const receive = (value: unknown) => { const event = safeDiagnostic(value); if (event) events.push(event); };
  const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  source.subscribe(receive);
  try {
    for (const cancelled of [false, true]) {
      const control = new AbortController();
      const reason = new Error("synthetic-private-error");
      const messages = [{ role: "user" as const,
        content: [{ kind: "text" as const, text: "private history ".repeat(6000) }] },
        { role: "assistant" as const, content: [{ kind: "text" as const, text: "Earlier answer" }] },
        { role: "user" as const, content: [{ kind: "text" as const, text: "Continue" }] }];
      await assert.rejects(compactContext({
        provider: { ...provider(), async send() { if (cancelled) control.abort(reason); throw reason; } },
        model: "fake-1", effort: "high", context: messages, turn: messages.slice(-1), nodeId: 1,
        coveredMessages: 0, lastInputTokens: 0, estimatedInputTokens: 60_000,
        policy: policyForContextWindow({ tokens: 64_000 }, 85), force: true,
        failLoudly: true, signal: control.signal,
      }), (error) => error === reason);
    }
    assert.deepEqual(events.map((e) => e.kind === "compaction" && [e.reason, e.outcome]),
      [["manual", "failed"], ["manual", "cancelled"]]);
    assert.doesNotMatch(JSON.stringify(events), /private/);
  } finally { source.unsubscribe(receive); }
});
