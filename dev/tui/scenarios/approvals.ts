import type { View } from "../../../src/tui/view.ts";
import type { LabState } from "../model.ts";
import * as edit from "../../../src/tui/editor.ts";
import { base, diffDetail } from "./shared.ts";
import type { Block } from "../../../src/tui/blocks.ts";
import { promptFor } from "../../../src/tui/approve.ts";
import { editPreview, commandApproval } from "../fixtures.ts";

export function approveEditScene(state: LabState): View {
  const call = {
    kind: "tool_call" as const,
    id: "preview",
    name: "edit_file",
    input: { path: editPreview.path },
  };
  return approvalScene(state, call, editPreview.path, [
    { kind: "user", text: editPreview.user },
    { kind: "reasoning", text: editPreview.reasoning, expanded: true },
    {
      kind: "tool",
      name: "edit_file",
      target: editPreview.path,
      right: `${editPreview.stat} · pending approval`,
      tone: "pending",
      body: editPreview.lines.map(diffDetail),
      expanded: true,
    },
  ]);
}

export function approveCommandScene(state: LabState): View {
  const call = {
    kind: "tool_call" as const,
    id: "command",
    name: "run_command",
    input: { command: commandApproval.command },
  };
  return approvalScene(state, call, commandApproval.command, [
    { kind: "user", text: commandApproval.user },
    { kind: "reasoning", text: commandApproval.reasoning },
    {
      kind: "tool",
      name: "run_command",
      target: commandApproval.command,
      right: "pending approval",
      tone: "pending",
      body: commandApproval.context.map((text) => ({ kind: "out", text })),
    },
  ]);
}

function approvalScene(
  state: LabState,
  call: Parameters<typeof promptFor>[0],
  target: string,
  blocks: Block[],
): View {
  const picker = promptFor(call, target, state.palette);
  return {
    ...base(state),
    blocks,
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "pick", picker: { ...picker, index: state.selected } },
  };
}

export function approveDeniedScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: editPreview.user },
      {
        kind: "tool",
        name: "edit_file",
        target: editPreview.path,
        right: `${editPreview.stat} · denied`,
        tone: "deny",
        body: editPreview.lines.map(diffDetail),
        expanded: state.expanded,
      },
    ],
    editor: edit.of("Keep the existing public error shape."),
    scroll: 0,
    feedback: { text: "Denied edit_file · feedback goes back to the model", tone: "info" },
  };
}
