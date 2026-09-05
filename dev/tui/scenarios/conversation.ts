import type { View } from "../../../src/tui/view.ts";
import type { LabState } from "../model.ts";
import * as edit from "../../../src/tui/editor.ts";
import { base, toolBlock, need, status } from "./shared.ts";
import { graphemes } from "../../../src/ui/width.ts";
import { matches } from "../../../src/tui/complete.ts";
import { conversation, literalPromptSample, markdownSample, reasoningSample } from "../fixtures.ts";
import { providerLabel } from "../../../src/provider-label.ts";

export function goldenScene(state: LabState): View {
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

export function conversationScene(state: LabState): View {
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

export function markdownScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: literalPromptSample },
      { kind: "answer", text: markdownSample },
    ],
    editor: edit.EMPTY,
    scroll: 0,
  };
}

export function reasoningScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: "Keep reasoning visible without letting it take over the transcript." },
      { kind: "reasoning", text: reasoningSample, live: true, expanded: state.expanded },
    ],
    editor: edit.EMPTY,
    scroll: 0,
    status: status(state, "Thinking"),
  };
}

export function feedbackScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [],
    editor: edit.of("keep this prompt in the composer"),
    scroll: 0,
    feedback: { text: `${providerLabel("anthropic")} needs an API key · /providers`, tone: "warn" },
    readiness: { text: `${providerLabel("anthropic")} needs an API key · /providers`, tone: "warn" },
  };
}

export function scrollScene(state: LabState): View {
  return {
    ...base(state),
    blocks: Array.from({ length: 80 }, (_, index) => ({
      kind: "answer", text: `Paragraph ${index + 1}: inspect the transcript while keeping the composer available.`,
    })),
    editor: edit.EMPTY, scroll: 0,
  };
}

export function steeringScene(state: LabState): View {
  return {
    ...reasoningScene(state),
    editor: edit.of("Keep the change focused"),
    steering: 0,
  };
}

export function reasoningStreamScene(state: LabState): View {
  const source = graphemes(reasoningSample);
  const arrived = Math.min(source.length, 20 + Math.floor(state.tick) * 6);
  return {
    ...base(state),
    blocks: [
      { kind: "user", text: "Inspect reasoning as it arrives, reflows, and settles." },
      { kind: "reasoning", text: source.slice(0, arrived).join(""), live: arrived < source.length, expanded: state.expanded },
    ],
    editor: edit.EMPTY, scroll: 0,
    ...(arrived < source.length ? { status: status(state, "Thinking"), steering: 0 } : {}),
  };
}
