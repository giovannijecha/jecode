// Isolated candidate, deliberately absent from the installed tool registry.
// Reconstruct facts from matched canonical tool results; never replay effects.
import type { Message, ToolCallBlock } from "../../src/types.ts";
import type { Tool } from "../../src/tools/types.ts";

type Step = { id: string; task: string; status: "pending" | "active" | "done" | "blocked" };
type Plan = { objective: string; steps: Step[] };
type Check = { callId: string; command: string; failed: boolean; output: string;
  laterFileEdits: number; laterCommands: number };

function text(value: unknown, limit: number): string {
  if (typeof value !== "string" || !value.trim() || value.length > limit) {
    throw new Error(`Expected nonempty text of at most ${limit} characters`);
  }
  return value;
}

function planInput(args: Record<string, unknown>): Plan {
  if (Object.keys(args).some(key => key !== "objective" && key !== "steps")) throw new Error("Unknown plan field");
  const objective = text(args.objective, 600);
  if (!Array.isArray(args.steps) || args.steps.length < 1 || args.steps.length > 8) {
    throw new Error("Use between one and eight steps");
  }
  const ids = new Set<string>();
  let active = 0;
  const steps = args.steps.map((value: unknown): Step => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("Invalid step");
    const step = value as Record<string, unknown>;
    if (Object.keys(step).some(key => !["id", "task", "status"].includes(key))) throw new Error("Unknown step field");
    const id = text(step.id, 32);
    if (!/^[a-zA-Z0-9_-]+$/u.test(id) || ids.has(id)) throw new Error("Step IDs must be unique identifiers");
    ids.add(id);
    const task = text(step.task, 300);
    const status = step.status;
    if (status !== "pending" && status !== "active" && status !== "done" && status !== "blocked") {
      throw new Error("Invalid step status");
    }
    if (status === "active" && ++active > 1) throw new Error("Use at most one active step");
    return { id, task, status };
  });
  return { objective, steps };
}

export function workState(history: readonly Message[]) {
  let plan: Plan | undefined;
  let guidanceSinceUpdate = 0;
  let latestGuidance = "";
  const checks: Check[] = [];
  // Calls can be announced together and their results checkpointed separately.
  // Forget consumed IDs; malformed/unmatched history cannot manufacture facts.
  const pending = new Map<string, ToolCallBlock>();
  for (const message of history) {
    for (const block of message.content) {
      if (message.role === "assistant" && block.kind === "tool_call") {
        if (pending.size < 32) pending.set(block.id, block);
      } else if (message.role === "user" && block.kind === "text") {
        guidanceSinceUpdate++;
        latestGuidance = block.text.slice(0, 600);
      } else if (message.role === "user" && block.kind === "tool_result") {
        const call = pending.get(block.id);
        pending.delete(block.id);
        if (call === undefined) continue;
        if (call.name === "work_state" && !block.isError && Object.keys(call.input).length > 0) {
          try { plan = planInput(call.input); guidanceSinceUpdate = 0; }
          catch { /* Old or malformed reports do not replace the last valid plan. */ }
        } else if (call.name === "run_command") {
          for (const check of checks) check.laterCommands++;
          checks.push({ callId: call.id, command: String(call.input.command ?? "").slice(0, 300),
            failed: block.isError, output: block.output.slice(-500), laterFileEdits: 0, laterCommands: 0 });
          if (checks.length > 8) checks.shift();
        } else if (call.name === "edit_file" || call.name === "write_file") {
          // Even a failed write can have an uncertain partial effect. Do not
          // promote earlier test results to evidence for a later revision.
          for (const check of checks) check.laterFileEdits++;
        }
      }
    }
  }
  return { plan: plan ?? null, guidanceSinceUpdate, latestGuidance, recentCommands: checks,
    evidenceBoundary: "Step status is model-reported. Command results are observed, not proof of all requirements. " +
      "Later file edits make earlier checks stale; later commands may also change files. External edits are not tracked." };
}

export function workStateTool(history: readonly Message[]): Tool {
  return {
    name: "work_state", dangerous: false, concurrency: "exclusive",
    description: "Optional checklist for complex multi-step work. Call with {} to read the current plan and recent command " +
      "evidence, including after compaction or resume. Otherwise replace the objective and steps with the user's requested " +
      "scope. Status is your report, not independent verification. Do not use for simple tasks or invent additional work.",
    input: { type: "object", properties: {
      objective: { type: "string", maxLength: 600 },
      steps: { type: "array", minItems: 1, maxItems: 8, items: { type: "object", properties: {
        id: { type: "string", maxLength: 32 }, task: { type: "string", maxLength: 300 },
        status: { type: "string", enum: ["pending", "active", "done", "blocked"] },
      }, required: ["id", "task", "status"], additionalProperties: false } },
    }, additionalProperties: false },
    async run(args, ctx) {
      ctx.signal?.throwIfAborted();
      const state = workState(history);
      const updated = Object.keys(args).length > 0;
      if (updated) { state.plan = planInput(args); state.guidanceSinceUpdate = 0; }
      return { output: JSON.stringify(state), summary: updated ? "plan updated" : "current work" };
    },
  };
}
