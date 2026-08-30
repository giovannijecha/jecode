// Realistic fixtures assembled entirely from production TUI contracts.

import { matches } from "../../src/tui/complete.ts";
import type { Block, Detail, ToolBlock } from "../../src/tui/blocks.ts";
import * as edit from "../../src/tui/editor.ts";
import { promptFor } from "../../src/tui/approve.ts";
import { settingsPicker } from "../../src/settings-command.ts";
import type { View } from "../../src/tui/view.ts";
import {
  conversation,
  editPreview,
  markdownSample,
  model,
  reasoningSample,
  toolTrace,
  workspace,
} from "./fixtures.ts";
import type { DiffLine, ToolFixture } from "./fixtures.ts";
import type { LabState } from "./model.ts";

export function sceneView(state: LabState): View {
  switch (state.scene) {
    case "tools":
      return toolsScene(state);
    case "diff":
      return diffScene(state);
    case "commands":
      return commandsScene(state);
    case "field":
      return fieldScene(state);
    case "markdown":
      return markdownScene(state);
    case "settings":
      return settingsScene(state);
    case "reasoning":
      return reasoningScene(state);
    case "feedback":
      return feedbackScene(state);
    default:
      return conversationScene(state);
  }
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
    footer: {
      workspace: `${workspace} (main)`,
      model: "claude-sonnet-5",
      effort: "high",
    },
    blocks: [],
    editor: edit.of("keep this prompt in the composer"),
    scroll: 0,
    feedback: { text: "Anthropic needs an API key · /settings", tone: "warn" },
  };
}

function settingsScene(state: LabState): View {
  return {
    ...base(state),
    footer: {
      workspace: `${workspace} (main)`,
      model: "deepseek-v4-flash:0731",
      effort: "high",
    },
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    modal: {
      kind: "pick",
      picker: settingsPicker(
        {
          provider: "ollama",
          model: "deepseek-v4-flash:0731",
          ollamaConnection: "cloud · ollama.com",
          effort: "high",
          maxTokens: 64000,
          maxSteps: 40,
          reducedMotion: false,
        },
        state.palette,
        state.selected,
        "~/.jecode/settings.json",
      ),
    },
  };
}

function base(state: LabState): Omit<View, "blocks" | "editor" | "scroll"> {
  return {
    pal: state.palette,
    footer: {
      workspace: `${workspace} (main)`,
      model,
      effort: "high",
    },
    spin: state.tick,
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

function toolsScene(state: LabState): View {
  const [found, failed, _read, pending] = toolTrace.tools;
  if (found === undefined || failed === undefined || pending === undefined) {
    throw new Error("tool fixture is incomplete");
  }
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: toolTrace.user },
      toolBlock(found, false),
      toolBlock(failed, state.expanded),
      { kind: "answer", text: toolTrace.diagnosis },
      toolBlock(pending, false),
    ],
    editor: edit.EMPTY,
    scroll: 0,
    status: "Tracing the failed assertion…",
  };
}

function diffScene(state: LabState): View {
  const call = {
    kind: "tool_call" as const,
    id: "preview",
    name: "edit_file",
    input: { path: editPreview.path },
  };
  const picker = promptFor(call, editPreview.path, state.palette);
  return {
    ...base(state),
    blocks: [
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
    ],
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "pick", picker: { ...picker, index: state.selected } },
  };
}

function commandsScene(state: LabState): View {
  const editor = edit.of("/");
  return {
    ...base(state),
    blocks: [
      { kind: "answer", text: "Type `/` to discover commands. The list narrows as you type." },
    ],
    editor,
    scroll: 0,
    menu: matches(editor.text),
    menuIndex: state.selected,
  };
}

function markdownScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: markdownSample }],
    editor: edit.EMPTY,
    scroll: 0,
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

function toolBlock(tool: ToolFixture, expanded: boolean): ToolBlock {
  return {
    kind: "tool",
    name: tool.name,
    target: tool.target,
    right: tool.result,
    tone: tool.tone,
    body: tool.detail?.map((text) => ({ kind: "out", text })),
    expanded,
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
