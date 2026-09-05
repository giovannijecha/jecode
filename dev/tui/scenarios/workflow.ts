// One inert conversation exercises the complete tool workflow.

import type { Block, Detail, ToolBlock } from "../../../src/tui/blocks.ts";
import * as edit from "../../../src/tui/editor.ts";
import type { View } from "../../../src/tui/view.ts";
import { graphemes } from "../../../src/ui/width.ts";
import type { ToolFixture } from "../fixtures.ts";
import type { LabState } from "../model.ts";
import { TICK_MS } from "../model.ts";
import { base, status, toolBlock } from "./shared.ts";
import { WORKFLOW_EVIDENCE as evidence } from "../fixtures/workflow.ts";

export const WORKFLOW_TIMES = {
  reasoningStartedMs: 400,
  reasoningSettledMs: 3_200,
  listWaitingMs: 3_200,
  listStartedMs: 4_000,
  listSettledMs: 5_600,
  readWaitingMs: 5_600,
  readStartedMs: 6_400,
  readSettledMs: 8_400,
  commandWaitingMs: 8_400,
  commandStartedMs: 9_200,
  commandSettledMs: 13_600,
  diagnosisStartedMs: 13_600,
  diagnosisSettledMs: 16_000,
  editStartedMs: 16_000,
  editSettledMs: 18_400,
  writeWaitingMs: 18_400,
  writeStartedMs: 19_200,
  writeSettledMs: 24_000,
  rerunWaitingMs: 24_000,
  rerunStartedMs: 24_800,
  rerunSettledMs: 28_800,
  answerStartedMs: 28_800,
  answerSettledMs: 33_200,
} as const;

export const WORKFLOW_DURATION_MS = 34_000;

export const WORKFLOW_MOMENTS = [
  { title: "Read summaries", time: WORKFLOW_TIMES.readSettledMs },
  { title: "Failing command", time: WORKFLOW_TIMES.commandSettledMs },
  { title: "Large edit running", time: 16_800 },
  { title: "New file running", time: 20_400 },
  { title: "Verified answer", time: WORKFLOW_TIMES.answerSettledMs },
] as const satisfies readonly { title: string; time: number }[];

export function workflowScene(state: LabState): View {
  // Evidence changes on the same 80 ms grid used by fixture playback.
  const now = Math.floor(state.tick) * TICK_MS;
  const times = WORKFLOW_TIMES;
  const blocks: Block[] = [{ kind: "user", text: evidence.user }];

  if (now >= times.reasoningStartedMs) {
    blocks.push({
      kind: "reasoning",
      text: streamed(evidence.reasoning, now, times.reasoningStartedMs, times.reasoningSettledMs),
      live: now < times.reasoningSettledMs,
      expanded: false,
    });
  }
  if (now >= times.listWaitingMs) {
    blocks.push(timedTool(evidence.list, now, times.listStartedMs, times.listSettledMs));
  }
  if (now >= times.readWaitingMs) {
    blocks.push(timedTool(evidence.read, now, times.readStartedMs, times.readSettledMs));
  }
  if (now >= times.commandWaitingMs) {
    blocks.push(timedTool(evidence.failedRun, now, times.commandStartedMs, times.commandSettledMs,
      outputAt(evidence.failedRun.detail, now, times.commandStartedMs, times.commandSettledMs)));
  }
  if (now >= times.diagnosisStartedMs) {
    blocks.push({
      kind: "reasoning",
      text: streamed(evidence.diagnosis, now, times.diagnosisStartedMs, times.diagnosisSettledMs),
      live: now < times.diagnosisSettledMs,
      expanded: false,
    });
  }
  if (now >= times.editStartedMs) {
    // Both mutations are already approved in the fixture; no I/O is performed.
    blocks.push(timedTool(evidence.edit, now, times.editStartedMs, times.editSettledMs, evidence.editDetails));
  }
  if (now >= times.writeWaitingMs) {
    blocks.push(timedTool(evidence.write, now, times.writeStartedMs, times.writeSettledMs, evidence.writeDetails));
  }
  if (now >= times.rerunWaitingMs) {
    blocks.push(timedTool(evidence.verify, now, times.rerunStartedMs, times.rerunSettledMs,
      outputAt(evidence.verify.detail, now, times.rerunStartedMs, times.rerunSettledMs)));
  }
  if (now >= times.answerStartedMs) {
    blocks.push({
      kind: "answer",
      text: streamed(evidence.answer, now, times.answerStartedMs, times.answerSettledMs),
    });
  }

  return {
    ...base(state), blocks, editor: edit.EMPTY, scroll: 0,
    ...(now < times.answerSettledMs ? { status: status(state, activity(now)), steering: 0 } : {}),
  };
}

function timedTool(
  fixture: ToolFixture,
  now: number,
  startedAt: number,
  settledAt: number,
  body?: readonly Detail[],
): ToolBlock {
  const waiting = now < startedAt;
  const settled = now >= settledAt;
  const tool = toolBlock(fixture, false);
  tool.tone = settled ? fixture.tone : "pending";
  tool.right = waiting ? "waiting" : settled ? fixture.result : "running";
  tool.body = body?.map((detail) => ({ ...detail })) ?? (settled ? tool.body : []);
  tool.startedAt = waiting || settled ? undefined : startedAt;
  tool.durationMs = settled ? settledAt - startedAt : undefined;
  return tool;
}

function outputAt(lines: readonly string[], now: number, startedAt: number, settledAt: number): Detail[] {
  const arrived = Math.max(0, Math.min(lines.length, Math.floor(
    lines.length * (now - startedAt) / (settledAt - startedAt),
  )));
  return lines.slice(0, arrived).map((text) => ({ kind: "out", text }));
}

function streamed(text: string, now: number, startedAt: number, settledAt: number): string {
  const source = graphemes(text);
  const progress = Math.max(0, Math.min(1, (now - startedAt) / (settledAt - startedAt)));
  return source.slice(0, Math.min(source.length, 12 + Math.floor((source.length - 12) * progress))).join("");
}

function activity(now: number): string {
  const times = WORKFLOW_TIMES;
  if (now < times.listWaitingMs) return "Thinking";
  if (now < times.listStartedMs) return "Waiting for list_dir";
  if (now < times.listSettledMs) return "Running list_dir";
  if (now < times.readStartedMs) return "Waiting for read_file";
  if (now < times.readSettledMs) return "Running read_file";
  if (now < times.commandStartedMs) return "Waiting for run_command";
  if (now < times.commandSettledMs) return "Running run_command";
  if (now < times.diagnosisSettledMs) return "Inspecting the failure";
  if (now < times.editSettledMs) return "Applying the approved edit";
  if (now < times.writeStartedMs) return "Preparing the approved write";
  if (now < times.writeSettledMs) return "Writing the cancellation cases";
  if (now < times.rerunStartedMs) return "Waiting for verification";
  if (now < times.rerunSettledMs) return "Verifying the retry boundaries";
  return "Responding";
}
