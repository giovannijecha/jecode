// Production TUI -> controller -> measured wire -> tools -> durable context -> resume.
import assert from "node:assert/strict";
import { channel } from "node:diagnostics_channel";
import { appendFile, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ConversationTree } from "../../src/conversation.ts";
import type { TurnNode } from "../../src/conversation.ts";
import { DurableSessionStore } from "../../src/sessions/store.ts";
import { SessionPersistence } from "../../src/sessions/runtime.ts";
import { runApp } from "../../src/tui/app.ts";
import { CONTEXT_DIAGNOSTIC_CHANNEL, safeDiagnostic } from "../../src/context/diagnostics.ts";
import type { ContextDiagnostic } from "../../src/context/diagnostics.ts";
import { session } from "../test-support/app.ts";
import { virtualScreen, waitFor, waitForIdle } from "../test-support/app-harness.ts";
import { contextFixture, SUMMARY } from "./context-fixture.ts";
import { CORPUS_SEED, mixedCorpus } from "./corpus.ts";
import { distribution } from "./samples.ts";
import { round } from "./report.ts";

export async function integratedContextProbe(columns = 60, reads = 12, termination: "interrupt" | "disconnect" = "interrupt") {
  assert.ok(Number.isSafeInteger(columns) && columns >= 40 && columns <= 120);
  assert.ok(Number.isSafeInteger(reads) && reads >= 8 && reads <= 16);
  assert.ok(termination === "interrupt" || termination === "disconnect");
  const root = await mkdtemp(join(tmpdir(), "jecode-context-integrated-"));
  const shutdown = new AbortController();
  const fixture = await contextFixture(reads, termination).catch(async error => {
    await rm(root, { recursive: true, force: true });
    throw error;
  });
  const diagnostics: ContextDiagnostic[] = [];
  const observations = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  const observe = (event: unknown) => { const safe = safeDiagnostic(event); if (safe) diagnostics.push(safe); };
  const saves: number[] = [];
  const effects: number[] = [];
  const source = mixedCorpus(8_192);
  let running: Promise<void> | undefined;
  let resumed: Promise<void> | undefined;
  let persistence: SessionPersistence | undefined;
  let nextPersistence: SessionPersistence | undefined;
  observations.subscribe(observe);
  const started = performance.now();
  try {
    const workspace = join(root, "workspace");
    await mkdir(workspace);
    const store = await DurableSessionStore.open(workspace, join(root, "sessions"));
    const makeSession = () => {
      const current = session(fixture.wire);
      current.config.root = workspace;
      current.tools = [{ name: "fixture_read", description: "Return a fixed source fixture and record its receipt.",
        input: { type: "object", properties: { index: { type: "integer" } }, required: ["index"] },
        dangerous: false, concurrency: "exclusive", async run(args) {
          const index = args["index"];
          assert.equal(index, effects.length, "tools must execute exactly once and in order");
          effects.push(index as number);
          await appendFile(join(workspace, "receipts.txt"), `${index}\n`);
          return { output: `fixture ${index}\n${source}` };
        } }];
      return current;
    };
    const first = makeSession();
    first.conversation = ConversationTree.empty().commit({ parentId: 0,
      createdAt: "2026-09-09T00:00:00.000Z", identity: { providerId: "openai", model: "gpt-5", effort: "high" },
      messages: [{ role: "user", content: [{ kind: "text", text: `Keep the public API stable.\n${mixedCorpus(32_768)}` }] },
        { role: "assistant", content: [{ kind: "text", text: "Earlier inspection recorded." }] }], blocks: [] }, "completed");
    persistence = first.persistence = SessionPersistence.fresh(store);
    const save = persistence.checkpoint.bind(persistence);
    persistence.checkpoint = async conversation => {
      const before = performance.now();
      await save(conversation);
      saves.push(performance.now() - before);
    };
    const screen = virtualScreen(columns);
    // Bound frame retention; keep the latest frame needed by the existing host checks.
    const paint = screen.environment.paint.paint;
    screen.environment.paint.paint = rows => { paint(rows); if (screen.frames.length > 2) screen.frames.shift(); };
    running = runApp(first, workspace, { ...screen.environment, shutdownSignal: shutdown.signal });
    // Make a rejected app promise observable while waiting for its state.
    let failure: unknown;
    running.catch(error => { failure = error; });
    const wait = (condition: () => boolean, label: string) => waitFor(() => {
      if (failure) throw failure;
      return condition();
    }, label, 30_000);
    const feed = await screen.input();
    feed("Inspect all fixture files and preserve the API.\r");
    await wait(() => fixture.state().waiting, "stream awaiting interruption");
    const interruption = performance.now();
    if (termination === "interrupt") feed("\x1b");
    await wait(() => first.conversation.activeNode?.settlement ===
      (termination === "interrupt" ? "interrupted" : "failed"), "termination checkpoint");
    const terminationMilliseconds = performance.now() - interruption;
    await waitForIdle(screen, "interrupted idle");
    assert.equal(effects.length, reads);
    assert.ok(fixture.state().summaries > 0 && fixture.state().summaries <= 2, "bounded automatic compaction");
    assert.ok(fixture.state().streamEvents > reads, "production stream events must reach the TUI");
    const anchor = first.conversation.activeNode?.context;
    assert.ok(anchor, "compaction anchor must be durable");
    const canonical = durableNodes(first.conversation.nodes);
    const results = first.conversation.history.flatMap(message => message.content).filter(block => block.kind === "tool_result");
    assert.equal(results.length, reads);
    results.forEach((result, index) => assert.equal(result.output, `fixture ${index}\n${source}`));
    assert.match(JSON.stringify(first.conversation.transcript), /Waiting for fixture interruption/);
    assert.ok(saves.length >= reads, "tool boundaries must checkpoint");
    const id = persistence.sessionId!;
    feed("/exit\r");
    await running;
    const resumeStart = performance.now();
    const saved = await SessionPersistence.resume(store, id);
    nextPersistence = saved.persistence;
    const resumeMilliseconds = performance.now() - resumeStart;
    assert.deepEqual(durableNodes(saved.conversation.nodes), canonical);
    assert.deepEqual(saved.conversation.activeNode?.context, anchor);
    const next = makeSession();
    next.persistence = nextPersistence;
    next.conversation = saved.conversation;
    const nextScreen = virtualScreen(columns);
    resumed = runApp(next, workspace, { ...nextScreen.environment, shutdownSignal: shutdown.signal });
    resumed.catch(error => { failure = error; });
    const resumeFeed = await nextScreen.input();
    await waitForIdle(nextScreen, "resume idle before input");
    const beforeResume = fixture.state();
    assert.equal(beforeResume.requests, reads + 1, "resume must not generate without input");
    assert.equal(effects.length, reads, "resume must not replay tools");
    fixture.resume();
    resumeFeed("Continue from the saved results.\r");
    await wait(() => next.conversation.activeNode?.settlement === "completed", "completed resumed turn");
    await waitForIdle(nextScreen, "completed resumed idle");
    assert.equal(fixture.state().requests, beforeResume.requests + 1);
    assert.equal(fixture.state().summaries, beforeResume.summaries, "resume must reuse the compacted projection");
    assert.deepEqual(durableNodes(next.conversation.nodes.slice(0, saved.conversation.nodes.length)), canonical);
    assert.equal(await readFile(join(workspace, "receipts.txt"), "utf8"), effects.map(value => `${value}\n`).join(""));
    assert.equal(effects.length, reads);
    assert.ok(diagnostics.some(event => event.kind === "compaction" && event.outcome === "accepted" && event.summaryChars === SUMMARY.length));
    resumeFeed("/exit\r");
    await resumed;
    return { workload: { seed: CORPUS_SEED, columns, reads, termination, initialCharacters: 32_768,
      outputCharactersPerRead: source.length, measurement: "production-responses-o200k-reference", transport: "loopback-websocket" },
      elapsedMilliseconds: round(performance.now() - started), terminationMilliseconds: round(terminationMilliseconds),
      resumeMilliseconds: round(resumeMilliseconds), measurement: distribution(fixture.measurements),
      checkpoint: distribution(saves), peakMeasuredInputTokens: Math.max(...fixture.measuredTokens),
      ...fixture.state(), canonicalResults: results.length, resumedWithoutReplay: true,
      preparation: distribution(diagnostics.flatMap(event =>
        event.kind === "request" && event.preparationMs !== undefined ? [event.preparationMs] : [])),
      passed: true };
  } finally {
    shutdown.abort();
    await Promise.allSettled([running, resumed]);
    observations.unsubscribe(observe);
    await persistence?.close();
    await nextPersistence?.close();
    await fixture.close();
    await rm(root, { recursive: true, force: true });
  }
}

function durableNodes(nodes: readonly TurnNode[]): unknown {
  // Compare canonical content, not provider-owned transport metadata that the
  // documented session format intentionally does not persist.
  return JSON.parse(JSON.stringify(nodes.map(node => ({ ...node,
    messages: node.messages.map(({ role, content, usage }) => ({ role, content, usage })),
  }))));
}
