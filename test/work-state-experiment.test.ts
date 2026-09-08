import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Message } from "../src/types.ts";
import { runTurn } from "../src/controller.ts";
import { ConversationTree } from "../src/conversation.ts";
import { DurableSessionStore } from "../src/sessions/store.ts";
import { SessionPersistence } from "../src/sessions/runtime.ts";
import { workState, workStateTool } from "../dev/experiments/work-state.ts";
import { assistantText, events, options, scripted } from "../dev/test-support/controller.ts";

const plan = { objective: "Fix the requested import bug", steps: [
  { id: "fix", task: "Preserve the import contract", status: "active" },
  { id: "check", task: "Run the regression checks", status: "pending" },
] };
function result(history: Message[], name: string, input: Record<string, unknown>, isError = false) {
  const id = `call-${history.length}`;
  history.push({ role: "assistant", content: [{ kind: "tool_call", id, name, input }] },
    { role: "user", content: [{ kind: "tool_result", id, isError, output: isError ? "failed" : "observed result" }] });
}

test("work-state reconstruction preserves a plan across context replacement and resume without replay", async () => {
  const history: Message[] = [];
  result(history, "work_state", plan);
  result(history, "run_command", { command: "npm test" });
  const resumed = JSON.parse(JSON.stringify(history)) as Message[];
  const original = structuredClone(resumed);
  const provider = scripted([{ role: "assistant", content: [
    { kind: "tool_call", id: "read-state", name: "work_state", input: {} },
  ] }, assistantText("Current work inspected.")]);
  const projection: Message[] = [{ role: "user", content: [{ kind: "text", text: "Compacted earlier history." }] }];
  await runTurn(resumed, options(provider, { tools: [workStateTool(resumed)] }), events(), undefined, projection);
  const toolResult = resumed.at(-2)!.content[0]!;
  assert.equal(toolResult.kind, "tool_result");
  if (toolResult.kind !== "tool_result") return;
  const state = JSON.parse(toolResult.output) as ReturnType<typeof workState>;
  assert.deepEqual(state.plan, plan);
  assert.equal(state.recentCommands[0]?.command, "npm test");
  assert.deepEqual(resumed.slice(0, original.length), original);
  assert.equal(provider.seen.length, 2);
});

test("evidence exposes later edits and commands without converting reports into proof", () => {
  const history: Message[] = [];
  result(history, "work_state", plan);
  result(history, "run_command", { command: "npm test" });
  result(history, "edit_file", { path: "src/import.js" });
  result(history, "run_command", { command: "npm run check" }, true);
  const state = workState(history);
  assert.equal(state.recentCommands[0]?.laterFileEdits, 1);
  assert.equal(state.recentCommands[0]?.laterCommands, 1);
  assert.equal(state.recentCommands[1]?.failed, true);
  assert.match(state.evidenceBoundary, /not proof/);
  assert.equal(state.plan?.steps[0]?.status, "active");
});

test("failed or unmatched updates cannot replace state, and new guidance requires review", () => {
  const history: Message[] = [];
  result(history, "work_state", plan);
  result(history, "work_state", { ...plan, objective: "unapproved expansion" }, true);
  history.push({ role: "assistant", content: [{ kind: "tool_call", id: "unfinished", name: "work_state",
    input: { ...plan, objective: "unfinished update" } }] });
  history.push({ role: "user", content: [{ kind: "text", text: "Only fix imports; leave formatting unchanged." }] });
  const state = workState(history);
  assert.deepEqual(state.plan, plan);
  assert.equal(state.guidanceSinceUpdate, 1);
  assert.match(state.latestGuidance, /leave formatting unchanged/);
  assert.equal(workState([]).plan, null);
});

test("work-state validates bounded atomic updates and cancellation", async () => {
  const tool = workStateTool([]);
  for (const args of [{ objective: "x" }, { ...plan, steps: [] }, { ...plan, extra: 1 },
    { ...plan, objective: "x".repeat(601) }, { ...plan, steps: [plan.steps[0], plan.steps[0]] },
    { ...plan, steps: [{ ...plan.steps[0], status: "verified" }] }]) {
    await assert.rejects(tool.run(args, { root: process.cwd() }));
  }
  const control = new AbortController();
  control.abort(new Error("cancelled"));
  await assert.rejects(tool.run(plan, { root: process.cwd(), signal: control.signal }), /cancelled/);
  assert.deepEqual(workState([]).plan, null);
});

test("work-state obeys controller permissions and does not force simple tasks to plan", async () => {
  const history: Message[] = [];
  const provider = scripted([{ role: "assistant", content: [
    { kind: "tool_call", id: "denied", name: "work_state", input: plan },
  ] }, assistantText("No changes.")]);
  await runTurn(history, options(provider, { tools: [workStateTool(history)], toolAllowed: () => false }), events());
  assert.equal(workState(history).plan, null);
  const simple = scripted([assistantText("The answer is 42.")]);
  await runTurn([], options(simple, { tools: [workStateTool([])] }), events());
  assert.equal(simple.seen.length, 1);
});

test("command evidence remains bounded on a long canonical history", () => {
  const history: Message[] = [];
  for (let index = 0; index < 100; index++) result(history, "run_command", { command: `check-${index}` });
  const state = workState(history);
  assert.equal(state.recentCommands.length, 8);
  assert.equal(state.recentCommands[0]?.command, "check-92");
  assert.equal(state.recentCommands.at(-1)?.laterCommands, 0);
});

test("the existing session codec retains work evidence behind a persisted compaction anchor", async () => {
  const root = await mkdtemp(join(tmpdir(), "jecode-work-state-"));
  const workspace = join(root, "workspace");
  await mkdir(workspace);
  const messages: Message[] = [{ role: "user", content: [{ kind: "text", text: "Fix imports" }] }];
  result(messages, "work_state", plan);
  result(messages, "run_command", { command: "npm test" });
  messages.push(assistantText("The first checks passed; implementation is pending."));
  const createdAt = new Date().toISOString();
  const conversation = ConversationTree.empty().commit({ parentId: 0, createdAt,
    identity: { providerId: "fake", model: "fixture", effort: "high" }, messages,
    blocks: [{ kind: "user", text: "Fix imports" }],
    context: { throughNodeId: 1, messageCount: messages.length, createdAt, summary: "Earlier work." },
  }, "completed");
  let resumed: Awaited<ReturnType<typeof SessionPersistence.resume>> | undefined;
  try {
    const store = await DurableSessionStore.open(workspace, join(root, "sessions"));
    const published = await store.publish(conversation);
    resumed = await SessionPersistence.resume(store, published.meta.id);
    assert.equal(resumed.conversation.contextHistory.length, 1);
    assert.doesNotMatch(JSON.stringify(resumed.conversation.contextHistory), /npm test|work_state/);
    assert.deepEqual(workState(resumed.conversation.history), workState(messages));
    assert.deepEqual(resumed.conversation.history, messages);
  } finally {
    await resumed?.persistence.close();
    await rm(root, { recursive: true, force: true });
  }
});
