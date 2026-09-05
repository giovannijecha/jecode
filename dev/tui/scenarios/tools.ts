import type { View } from "../../../src/tui/view.ts";
import type { LabState } from "../model.ts";
import * as edit from "../../../src/tui/editor.ts";
import { base, toolBlock, commandBlock, diffDetail, need, status } from "./shared.ts";
import { conversation, toolTrace, suiteRun, outputCapDiff } from "../fixtures.ts";
import { TICK_MS } from "../model.ts";

const INITIAL_OUTPUT_ROWS = 7;
const OUTPUT_TICKS = 3;
export const TOOL_SETTLED_MS = 3_200;

export function lifecycleScene(state: LabState): View {
  const now = Math.floor(state.tick) * TICK_MS;
  const waiting = now < 800;
  const done = now >= TOOL_SETTLED_MS;
  const tool = commandBlock(
    waiting ? [] : suiteRun.output.slice(0, done ? undefined : Math.max(1, Math.floor((now - 800) / 120))),
    done ? "ok" : "pending", waiting ? "waiting" : done ? "exit 0" : "running", state.expanded,
    done ? 2_400 : undefined,
  );
  if (!waiting && !done) tool.startedAt = 800;
  return {
    ...base(state),
    blocks: [{ kind: "user", text: suiteRun.user }, tool],
    editor: edit.EMPTY, scroll: 0,
    ...(done ? {} : { status: status(state, waiting ? "Waiting" : "Running run_command"), steering: 0 }),
  };
}

export function toolsLiveScene(state: LabState): View {
  const [read, search, editing] = conversation.tools;
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: conversation.user },
      { kind: "reasoning", text: conversation.reasoning },
      toolBlock(need(read), false),
      toolBlock(need(search), false),
      {
        ...toolBlock(need(editing), false),
        tone: "pending",
        right: "running",
        startedAt: 0,
      },
    ],
    editor: edit.EMPTY,
    scroll: 0,
    status: status(state, "Applying the bounded reader"),
  };
}

export function toolsTraceScene(state: LabState): View {
  const [found, failed, pending] = toolTrace.tools;
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: toolTrace.user },
      toolBlock(need(found), false),
      toolBlock(need(failed), state.expanded),
      { kind: "answer", text: toolTrace.diagnosis },
      { ...toolBlock(need(pending), false), right: "running", startedAt: 0 },
    ],
    editor: edit.EMPTY,
    scroll: 0,
    status: status(state, "Tracing the failed assertion"),
  };
}

export function toolsOutputScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: suiteRun.user },
      commandBlock(suiteRun.output, "ok", suiteRun.result, state.expanded, suiteRun.durationMs),
      { kind: "answer", text: suiteRun.answer },
    ],
    editor: edit.EMPTY,
    scroll: 0,
  };
}

export function toolsStreamScene(state: LabState): View {
  const arrived = Math.min(suiteRun.output.length, INITIAL_OUTPUT_ROWS + Math.floor(state.tick / OUTPUT_TICKS));
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: suiteRun.user },
      {
        ...commandBlock(suiteRun.output.slice(0, arrived), "pending", "running", state.expanded),
        startedAt: 0,
      },
    ],
    editor: edit.EMPTY,
    scroll: 0,
    status: status(state, "Running run_command"),
  };
}

export function toolsDiffScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: outputCapDiff.user },
      { kind: "reasoning", text: outputCapDiff.reasoning },
      {
        kind: "tool",
        name: "edit_file",
        target: outputCapDiff.path,
        right: `${outputCapDiff.stat} · applied`,
        tone: "ok",
        body: outputCapDiff.lines.map(diffDetail),
        expanded: state.expanded,
      },
      { kind: "answer", text: outputCapDiff.answer },
    ],
    editor: edit.EMPTY,
    scroll: 0,
  };
}
