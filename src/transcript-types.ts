// Provider-neutral transcript vocabulary.
//
// The conversation domain stores these settled semantic blocks so a resumed
// session can rebuild the same screen without persisting terminal escape
// sequences or renderer state.

export type ToolTone = "pending" | "ok" | "fail" | "deny";
export type NoticeTone = "info" | "warn" | "error";

export type Emphasis = {
  readonly start: number;
  readonly length: number;
};

export type Detail =
  | { kind: "out"; text: string }
  | {
      kind: "keep" | "add" | "del";
      text: string;
      oldLine?: number;
      newLine?: number;
      emphasis?: Emphasis;
    }
  | { kind: "gap"; text: string };

export type UserBlock = { kind: "user"; text: string };
export type AnswerBlock = { kind: "answer"; text: string };
export type ReasoningBlock = {
  kind: "reasoning";
  text: string;
  /** True only while stream events are still filling this block. */
  live?: boolean;
  /** Full text is an explicit detail view; the default is a bounded preview. */
  expanded?: boolean;
};
export type ToolBlock = {
  kind: "tool";
  name: string;
  target: string;
  right: string;
  tone: ToolTone;
  body?: Detail[];
  expanded?: boolean;
  /** Wall-clock start for the live elapsed label; absent while approval waits. */
  startedAt?: number;
  /** Settled execution duration. Durable evidence; never used as animation state. */
  durationMs?: number;
};
export type NoticeBlock = { kind: "notice"; text: string; tone: NoticeTone };

export type TranscriptBlock =
  | UserBlock
  | AnswerBlock
  | ReasoningBlock
  | ToolBlock
  | NoticeBlock;
