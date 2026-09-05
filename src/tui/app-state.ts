// Mutable screen state shared by the shell, input, and foreground workflows.

import type { Activity } from "./activity.ts";
import type { Block } from "./blocks.ts";
import type { Completion } from "./complete.ts";
import type { Editor } from "./editor.ts";
import * as edit from "./editor.ts";
import type { Feedback } from "./feedback.ts";
import type { Open } from "./overlay.ts";
import type { FooterInfo } from "./components/footer.ts";

export type AppState = {
  blocks: Block[];
  editor: Editor;
  scroll: number;
  follow: boolean;
  unseen: number;
  lastMaxScroll: number;
  past: string[];
  recall: number;
  draft: string;
  feedback?: Feedback;
  open?: Open;
  /** Tool approval takes focus without discarding an open command menu. */
  approval?: Open;
  activity?: Activity;
  command?: Activity;
  /** The running turn keeps its original model and effort until settlement. */
  turnFooter?: FooterInfo;
  /** Pending guidance count while the active model turn still accepts steering. */
  steering?: number;
  closeWhenIdle: boolean;
  /** Last node created by a real turn, before any temporary timeline selection. */
  committedNodeId: number;
  /** The slash-command menu, held apart from the text being edited. */
  completing?: Completion;
  /** An oversized input cannot be submitted as a silently shortened prompt. */
  promptRejected: boolean;
};

export function appState(): AppState {
  return {
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    follow: true,
    unseen: 0,
    lastMaxScroll: 0,
    past: [],
    recall: -1,
    draft: "",
    closeWhenIdle: false,
    committedNodeId: 0,
    promptRejected: false,
  };
}
