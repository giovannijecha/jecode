// A compact, read-only projection of the durable conversation tree.
//
// Selecting a node changes only the in-memory path. The next real user turn
// is what persists a branch, so opening or cancelling this control plane can
// never create empty history.

import type { ConversationTree, TurnNode } from "./conversation.ts";
import type { Session } from "./session.ts";
import type { Palette } from "./ui/theme.ts";
import { usageFromHistory } from "./usage.ts";
import type { Picker } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";

export type Timeline = Readonly<{
  picker: Picker;
  nodeIds: readonly number[];
}>;

export function timelinePicker(conversation: ConversationTree, palette: Palette): Timeline {
  const entries = timelineEntries(conversation);
  const selectedId = conversation.latestResumable()?.activeNodeId ?? 0;
  const index = Math.max(0, entries.findIndex((entry) => entry.node.id === selectedId));
  return Object.freeze({
    picker: {
      title: heading("timeline", "conversation tree", palette),
      searchable: true,
      query: "",
      visible: 8,
      options: entries.map((entry) => ({
        label: `${entry.prefix}${preview(entry.node)}`,
        hint: stamp(entry.node.createdAt),
        ...timelineValue(entry.node, selectedId),
      })),
      index,
    },
    nodeIds: Object.freeze(entries.map((entry) => entry.node.id)),
  });
}

export async function selectTimeline(
  session: Session,
  choose: (picker: Picker) => Promise<number | undefined>,
): Promise<boolean> {
  const timeline = timelinePicker(session.conversation, session.palette);
  if (timeline.nodeIds.length === 0) return false;
  const index = await choose(timeline.picker);
  const nodeId = index === undefined ? undefined : timeline.nodeIds[index];
  if (nodeId === undefined || nodeId === session.conversation.activeNodeId) return false;

  session.conversation = session.conversation.select(nodeId);
  session.usage = usageFromHistory(session.conversation.history);
  return true;
}

type Entry = Readonly<{ node: TurnNode; prefix: string }>;

function timelineEntries(conversation: ConversationTree): Entry[] {
  const resumable = conversation.nodes.filter((node) => node.settlement !== "checkpointed");
  const resumableIds = new Set(resumable.map((node) => node.id));
  const children = new Map<number, TurnNode[]>();
  for (const node of resumable) {
    let parentId = node.parentId;
    while (parentId !== 0 && !resumableIds.has(parentId)) {
      parentId = conversation.node(parentId)?.parentId ?? 0;
    }
    const siblings = children.get(parentId) ?? [];
    siblings.push(node);
    children.set(parentId, siblings);
  }

  const entries: Entry[] = [];
  const visit = (parentId: number, lanes: readonly boolean[]): void => {
    const siblings = children.get(parentId) ?? [];
    for (let index = 0; index < siblings.length; index++) {
      const node = siblings[index] as TurnNode;
      const forks = siblings.length > 1;
      const last = index === siblings.length - 1;
      const hidden = Math.max(0, lanes.length - 4);
      const lanePrefix = `${hidden === 0 ? "" : "…  "}${lanes.slice(hidden)
        .map((closed) => closed ? "   " : "│  ").join("")}`;
      entries.push({
        node,
        prefix: `${lanePrefix}${forks ? (last ? "└─ " : "├─ ") : "• "}`,
      });
      visit(node.id, forks ? [...lanes, last] : lanes);
    }
  };
  visit(0, []);
  return entries;
}

function timelineValue(node: TurnNode, selectedId: number): { value?: string } {
  const state = node.settlement === "completed" ? undefined : node.settlement;
  const active = node.id === selectedId ? "active" : undefined;
  const value = [state, active].filter((part) => part !== undefined).join(" · ");
  return value === "" ? {} : { value };
}

function preview(node: TurnNode): string {
  for (const message of node.messages) {
    if (message.role !== "user") continue;
    const text = message.content.find((block) => block.kind === "text")?.text
      .replace(/\s+/gu, " ").trim();
    if (text !== undefined && text !== "") return text.slice(0, 160);
  }
  return "Untitled turn";
}

function stamp(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return value.replace("T", " ").slice(0, 16);
  const two = (part: number): string => String(part).padStart(2, "0");
  return `${two(date.getHours())}:${two(date.getMinutes())}`;
}
