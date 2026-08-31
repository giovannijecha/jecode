// Realistic fixtures assembled entirely from production TUI contracts.

import { matches } from "../../src/tui/complete.ts";
import type { Block, Detail, ToolBlock } from "../../src/tui/blocks.ts";
import * as edit from "../../src/tui/editor.ts";
import { promptFor } from "../../src/tui/approve.ts";
import type { Picker } from "../../src/tui/picker.ts";
import { heading } from "../../src/tui/picker.ts";
import { settingsPicker } from "../../src/settings-command.ts";
import type { View } from "../../src/tui/view.ts";
import {
  commandApproval,
  conversation,
  editPreview,
  markdownSample,
  model,
  modelChoices,
  modelQuery,
  outputCapDiff,
  reasoningSample,
  suiteRun,
  toolTrace,
  workspace,
} from "./fixtures.ts";
import type { DiffLine, ToolFixture } from "./fixtures.ts";
import type { LabState } from "./model.ts";

export function sceneView(state: LabState): View {
  switch (state.scene) {
    case "golden": return goldenScene(state);
    case "tools-live": return toolsLiveScene(state);
    case "tools-trace": return toolsTraceScene(state);
    case "tools-output": return toolsOutputScene(state);
    case "tools-stream": return toolsStreamScene(state);
    case "tools-diff": return toolsDiffScene(state);
    case "approve-edit": return approveEditScene(state);
    case "approve-command": return approveCommandScene(state);
    case "approve-denied": return approveDeniedScene(state);
    case "menu-commands": return commandsScene(state);
    case "menu-search": return searchScene(state);
    case "menu-settings": return settingsScene(state);
    case "help": return helpScene(state);
    case "field": return fieldScene(state);
    case "markdown": return markdownScene(state);
    case "reasoning": return reasoningScene(state);
    case "feedback": return feedbackScene(state);
    default: return conversationScene(state);
  }
}

/** One review frame that exercises the hierarchy from user turn to next input. */
function goldenScene(state: LabState): View {
  const [read, search] = conversation.tools;
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: conversation.user },
      { kind: "reasoning", text: conversation.reasoning },
      toolBlock(need(read), false),
      toolBlock(need(search), false),
      { kind: "answer", text: "The request path is mapped. I can harden the boundary without changing provider behavior." },
    ],
    editor: edit.of("/"),
    scroll: 0,
    menu: matches("/"),
    menuIndex: state.selected,
  };
}

function conversationScene(state: LabState): View {
  const answer = [conversation.answer[0], "", ...conversation.answer.slice(1).map((line) => `- ${line}`)].join("\n");
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: conversation.user },
      { kind: "answer", text: answer },
    ],
    editor: edit.EMPTY,
    scroll: 0,
  };
}

function toolsLiveScene(state: LabState): View {
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
    status: "Applying the bounded reader",
  };
}

function toolsTraceScene(state: LabState): View {
  const [found, failed, _read, pending] = toolTrace.tools;
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
    status: "Tracing the failed assertion",
  };
}

function toolsOutputScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: suiteRun.user },
      commandBlock(suiteRun.output, "ok", suiteRun.result, state.expanded),
      { kind: "answer", text: suiteRun.answer },
    ],
    editor: edit.EMPTY,
    scroll: 0,
  };
}

function toolsStreamScene(state: LabState): View {
  const arrived = Math.min(suiteRun.output.length, 7 + Math.floor(state.tick / 3));
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
    status: "Running run_command",
  };
}

function toolsDiffScene(state: LabState): View {
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

function approveEditScene(state: LabState): View {
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

function approveCommandScene(state: LabState): View {
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

function approveDeniedScene(state: LabState): View {
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

function commandsScene(state: LabState): View {
  const editor = edit.of("/");
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Type `/` to discover commands. The list narrows as you type." }],
    editor,
    scroll: 0,
    menu: matches(editor.text),
    menuIndex: state.selected,
  };
}

function searchScene(state: LabState): View {
  const hits = matchingModels();
  const picker: Picker = {
    title: heading("model", "switch the model for this session", state.palette),
    options: modelChoices,
    searchable: true,
    query: modelQuery,
    index: hits[state.selected % Math.max(1, hits.length)] ?? 0,
  };
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Long pickers use the same writable prompt as every other dock input." }],
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "pick", picker },
  };
}

function matchingModels(): number[] {
  const query = modelQuery.toLocaleLowerCase();
  return modelChoices.flatMap((option, index) =>
    `${option.label} ${option.hint}`.toLocaleLowerCase().includes(query) ? [index] : []
  );
}

function settingsScene(state: LabState): View {
  return {
    ...base(state),
    footer: { workspace: `${workspace} (main)`, model: "deepseek-v4-flash:0731", effort: "high" },
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    modal: {
      kind: "pick",
      picker: settingsPicker({
        provider: "ollama",
        model: "deepseek-v4-flash:0731",
        ollamaConnection: "cloud · ollama.com",
        effort: "high",
        maxTokens: 64000,
        maxSteps: 40,
        reducedMotion: false,
      }, state.palette, state.selected, "~/.jecode/settings.json"),
    },
  };
}

function helpScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Operational reference stays outside the conversation." }],
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "help" },
  };
}

function fieldScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Credentials stay outside the workspace and transcript." }],
    editor: edit.EMPTY,
    scroll: 0,
    modal: {
      kind: "type",
      field: {
        title: [
          { text: "paste key", fg: state.palette.ink.attention, bold: true },
          { text: "  OLLAMA_API_KEY", fg: state.palette.ink.fg },
        ],
        right: "enter ok · esc skip",
        editor: edit.of("masked-demo-value"),
        secret: true,
        note: "Environment values take precedence.",
      },
    },
  };
}

function markdownScene(state: LabState): View {
  return { ...base(state), blocks: [{ kind: "answer", text: markdownSample }], editor: edit.EMPTY, scroll: 0 };
}

function reasoningScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: "Keep reasoning visible without letting it take over the transcript." },
      { kind: "reasoning", text: reasoningSample, live: true, expanded: state.expanded },
    ],
    editor: edit.EMPTY,
    scroll: 0,
    status: "Thinking",
  };
}

function feedbackScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [],
    editor: edit.of("keep this prompt in the composer"),
    scroll: 0,
    feedback: { text: "Anthropic needs an API key · /settings", tone: "warn" },
  };
}

function base(state: LabState): Omit<View, "blocks" | "editor" | "scroll"> {
  return {
    pal: state.palette,
    footer: { workspace: `${workspace} (main)`, model, effort: "high" },
    spin: state.tick,
    now: 4_200 + state.tick * 80,
  };
}

function commandBlock(
  output: readonly string[],
  tone: "ok" | "pending",
  right: string,
  expanded: boolean,
): ToolBlock {
  return {
    kind: "tool",
    name: "run_command",
    target: suiteRun.command,
    right,
    tone,
    body: output.map((text) => ({ kind: "out", text })),
    expanded,
  };
}

function toolBlock(tool: ToolFixture, expanded: boolean): ToolBlock {
  return {
    kind: "tool",
    name: tool.name,
    target: tool.target,
    right: tool.tone === "pending" ? "running" : tool.result,
    tone: tool.tone,
    body: tool.detail?.map((text) => ({ kind: "out", text })),
    expanded,
    ...(tool.tone === "pending" ? { startedAt: 0 } : {}),
  };
}

function diffDetail(line: DiffLine): Detail {
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

function need<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("fixture is incomplete");
  return value;
}
