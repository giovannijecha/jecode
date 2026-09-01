import type { Palette } from "../../ui/theme.ts";
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
    return withUnseen([
      { text: info.status, fg: pal.ink.muted },
      { text: " · esc to interrupt", fg: pal.ink.dim, optional: true },
    ], info.unseen, pal);
  }
  if (info.feedback !== undefined) {
    return withUnseen(feedbackSegments(info.feedback, pal), info.unseen, pal);
  }
  if (info.unseen > 0) return unseenSegments(info.unseen, pal);
  return info.readiness === undefined ? [] : feedbackSegments(info.readiness, pal);
}

function feedbackSegments(feedback: Feedback, pal: Palette): Seg[] {
  if (feedback.tone === "info") {
    return [{ text: feedback.text, fg: pal.ink.muted }];
  }

  const mark = feedback.tone === "error" ? "×" : "!";
  const markColor = feedback.tone === "error" ? pal.ink.removed : pal.ink.attention;
  const textColor = feedback.tone === "error" ? pal.ink.removed : pal.ink.muted;
  return [
    { text: `${mark} `, fg: markColor, bold: true },
    { text: feedback.text, fg: textColor },
  ];
}

function withUnseen(status: Seg[], unseen: number, pal: Palette): Seg[] {
  return unseen === 0
    ? status
    : [...status, { text: " · ", fg: pal.ink.dim }, ...unseenSegments(unseen, pal)];
}

function unseenSegments(unseen: number, pal: Palette): Seg[] {
  return [{ text: `${unseen} new ↓`, fg: pal.accent, bold: true }];
}
