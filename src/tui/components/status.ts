import type { Palette, RGB } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import type { Feedback } from "../feedback.ts";

export type StatusInfo = {
  status: string | undefined;
  feedback: Feedback | undefined;
  readiness: Feedback | undefined;
  unseen: number;
};

/** Styled content for the footer's replaceable right-hand status channel. */
export function renderStatus(info: StatusInfo, pal: Palette): Seg[] {
  const urgent = info.feedback?.tone === "warn" || info.feedback?.tone === "error"
    ? info.feedback
    : undefined;
  if (urgent !== undefined) return withUnseen(feedbackSegments(urgent, pal), info.unseen, pal);
  if (info.status !== undefined) {
    return withUnseen([{ text: "esc to interrupt", fg: pal.ink.muted }], info.unseen, pal);
  }
  if (info.feedback !== undefined) {
    return withUnseen(feedbackSegments(info.feedback, pal), info.unseen, pal);
  }
  if (info.unseen > 0) return unseenSegments(info.unseen, pal);
  return info.readiness === undefined ? [] : feedbackSegments(info.readiness, pal);
}

function feedbackSegments(feedback: Feedback, pal: Palette): Seg[] {
  const mark = feedback.tone === "error" ? "×" : feedback.tone === "warn" ? "!" : "·";
  const markColor: Record<Feedback["tone"], RGB> = {
    info: pal.accent,
    warn: pal.ink.attention,
    error: pal.ink.removed,
  };
  const textColor = feedback.tone === "error" ? pal.ink.removed : pal.ink.muted;
  return [
    { text: `${mark} `, fg: markColor[feedback.tone], bold: true },
    { text: feedback.text, fg: textColor },
  ];
}

function withUnseen(status: Seg[], unseen: number, pal: Palette): Seg[] {
  return unseen === 0
    ? status
    : [...status, { text: " · ", fg: pal.ink.muted }, ...unseenSegments(unseen, pal)];
}

function unseenSegments(unseen: number, pal: Palette): Seg[] {
  return [{ text: `${unseen} new ↓`, fg: pal.accent, bold: true }];
}
