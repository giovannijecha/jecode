import type { View } from "../../../src/tui/view.ts";
import type { LabState } from "../model.ts";
import * as edit from "../../../src/tui/editor.ts";
import { base } from "./shared.ts";
import { ConversationTree } from "../../../src/conversation.ts";
import { timelinePicker } from "../../../src/timeline.ts";
import { resumePicker } from "../../../src/tui/resume.ts";
import type { SessionCatalogEntry } from "../../../src/sessions/store.ts";

export function resumeScene(state: LabState): View {
  const candidates: SessionCatalogEntry[] = [
    {
      id: "session-durable",
      createdAt: "2026-09-01T08:00:00.000Z",
      updatedAt: "2026-09-01T09:35:00.000Z",
      turns: 12,
      preview: "Harden durable sessions across Windows, WSL, and interrupted tool turns",
      active: false,
    },
    {
      id: "session-footer",
      createdAt: "2026-08-31T14:00:00.000Z",
      updatedAt: "2026-08-31T16:42:00.000Z",
      turns: 4,
      preview: "Refine the transcript footer and active status",
      active: false,
    },
    {
      id: "session-providers",
      createdAt: "2026-08-30T10:00:00.000Z",
      updatedAt: "2026-08-30T11:08:00.000Z",
      turns: 2,
      preview: "Review provider and model controls",
      active: false,
    },
  ];
  const picker = resumePicker(candidates, state.palette);
  return {
    ...base(state),
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "pick", picker: { ...picker, index: state.selected } },
  };
}

export function timelineScene(state: LabState): View {
  const identity = { providerId: "openai-codex", model: "gpt-5.6-sol", effort: "high" };
  const append = (
    tree: ConversationTree,
    parentId: number,
    user: string,
    answer: string,
    minute: string,
  ): ConversationTree => tree.commit({
    parentId,
    createdAt: `2026-09-02T10:${minute}:00.000Z`,
    identity,
    messages: [
      { role: "user", content: [{ kind: "text", text: user }] },
      { role: "assistant", content: [{ kind: "text", text: answer }] },
    ],
    blocks: [{ kind: "user", text: user }, { kind: "answer", text: answer }],
  }, "completed");
  const first = append(ConversationTree.empty(), 0, "Design durable sessions", "Session identity is stable.", "00");
  const main = append(first, 1,
    "Add context compaction while preserving complete messages, tool evidence, exported history, and branch-local summaries across session recovery",
    "Compaction is model-aware.", "04");
  const alternate = append(main.select(1), 1,
    "Explore a smaller storage format with bounded reads, atomic checkpoints, recoverable indexes, and clear failures that preserve the existing conversation",
    "The existing format stays canonical.", "07");
  const tree = append(alternate.select(2), 2, "Add timeline navigation", "Branching is deferred.", "12").select(3);
  const timeline = timelinePicker(tree, state.palette).picker;
  return {
    ...base(state),
    blocks: tree.transcript,
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "pick", picker: { ...timeline, index: state.selected } },
  };
}
