import type { View } from "../../../src/tui/view.ts";
import type { ToolBlock, Detail } from "../../../src/tui/blocks.ts";
import type { LabState } from "../model.ts";
import { TICK_MS } from "../model.ts";
import { model, workspace, suiteRun } from "../fixtures.ts";
import type { ToolFixture, DiffLine } from "../fixtures.ts";

export function base(state: LabState): Omit<View, "blocks" | "editor" | "scroll"> {
  return {
    pal: state.palette,
    footer: { workspace: `${workspace} (main)`, provider: "Anthropic API", model, effort: "high" },
    now: state.tick * TICK_MS,
  };
}

export function status(state: LabState, label: string): string {
  return `${label} · ${Math.floor(state.tick * TICK_MS / 1_000)}s`;
}

export function commandBlock(
  output: readonly string[],
  tone: "ok" | "pending",
  right: string,
  expanded: boolean,
  durationMs?: number,
): ToolBlock {
  return {
    kind: "tool",
    name: "run_command",
    target: suiteRun.command,
    right,
    tone,
    body: output.map((text) => ({ kind: "out", text })),
    expanded,
    ...(durationMs === undefined ? {} : { durationMs }),
  };
}

export function toolBlock(tool: ToolFixture, expanded: boolean): ToolBlock {
  return {
    kind: "tool",
    name: tool.name,
    target: tool.target,
    right: tool.tone === "pending" ? "running" : tool.result,
    tone: tool.tone,
    body: tool.detail?.map((text) => ({ kind: "out", text })),
    expanded,
    ...(tool.tone === "pending" ? { startedAt: 0 } : {}),
    ...(tool.durationMs === undefined ? {} : { durationMs: tool.durationMs }),
  };
}

export function diffDetail(line: DiffLine): Detail {
  if (line.kind === "gap") return { kind: "gap", text: line.text };
  const start = line.emphasis === undefined ? -1 : line.text.indexOf(line.emphasis);
  return {
    kind: line.kind,
    text: line.text,
    oldLine: line.oldLine,
    newLine: line.newLine,
    ...(start < 0 || line.emphasis === undefined
      ? {}
      : { emphasis: { start, length: line.emphasis.length } }),
  };
}

export function need<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("fixture is incomplete");
  return value;
}
