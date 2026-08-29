// Operational feedback belongs in the footer, not in the conversation.

import type { Session } from "../session.ts";
import type { Block, NoticeTone } from "./blocks.ts";

export type Feedback = {
  text: string;
  tone: NoticeTone;
  /** Omitted feedback stays until the next key, which is useful for blockers. */
  timeoutMs?: number;
};

export type FeedbackController = {
  show(feedback: Feedback): void;
  dismiss(): void;
  close(): void;
};

const INFO_MS = 2_800;
const WARN_MS = 4_200;
const ERROR_MS = 6_000;

/** Own replacement and expiry without leaking timers into the TUI shell. */
export function feedbackController(
  changed: (feedback: Feedback | undefined) => void,
): FeedbackController {
  let current: Feedback | undefined;
  let timer: NodeJS.Timeout | undefined;

  const clearTimer = (): void => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };

  return {
    show(feedback) {
      clearTimer();
      current = feedback;
      changed(feedback);
      if (feedback.timeoutMs === undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        if (current !== feedback) return;
        current = undefined;
        changed(undefined);
      }, feedback.timeoutMs);
    },
    dismiss() {
      clearTimer();
      if (current === undefined) return;
      current = undefined;
      changed(undefined);
    },
    close() {
      clearTimer();
      current = undefined;
    },
  };
}

/** Turn a command notice into one replaceable message in the footer status channel. */
export function commandFeedback(block: Block): Feedback | undefined {
  if (block.kind !== "notice") return undefined;
  return {
    text: block.text,
    tone: block.tone,
    timeoutMs: block.tone === "info" ? INFO_MS : block.tone === "warn" ? WARN_MS : ERROR_MS,
  };
}

/** Explain why a model turn cannot start, without exposing configuration internals. */
export function turnBlocker(session: Session): Feedback | undefined {
  const blocked = session.provider.blocked();
  if (blocked !== undefined && !blocked.startsWith(`${session.provider.keyVar} `)) {
    return { text: blocked, tone: "error" };
  }
  if (blocked !== undefined) {
    return {
      text: `${providerName(session.provider.id)} needs an API key · /settings`,
      tone: "warn",
    };
  }
  if (session.model === "") {
    return {
      text: `${providerName(session.provider.id)} needs a model · /models`,
      tone: "warn",
    };
  }
  return undefined;
}

function providerName(id: string): string {
  return id === "" ? "Provider" : `${id[0]?.toUpperCase() ?? ""}${id.slice(1)}`;
}
