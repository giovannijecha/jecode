// Jeco as terminal-native pixel art. Two source pixels share one terminal cell:
// the upper pixel is the foreground of a half block and the lower pixel is its
// background. The result stays compact without relying on image protocols.

import type { RGB } from "../../ui/theme.ts";
import { paint } from "../../ui/render.ts";
import type { ActivityKind } from "../activity.ts";
import type { Block, NoticeTone } from "./types.ts";

export type MascotState = "idle" | "thinking" | "typing" | "success" | "warning" | "error";

export type MascotSignal = {
  activityKind?: ActivityKind;
  status?: string;
  feedbackTone?: NoticeTone;
  readinessTone?: NoticeTone;
  blocks: readonly Block[];
};

export const MASCOT_COLS = 24;
export const MASCOT_ROWS = 8;

type Pixel = "." | "O" | "B" | "S" | "W" | "K" | "T" | "G" | "Y" | "R";
type Dot = readonly [x: number, y: number, pixel: Exclude<Pixel, ".">];

// Brand colours stay fixed: semantic state changes the prop beside Jeco, not
// the steel-blue character itself.
const COLORS: Record<Exclude<Pixel, ".">, RGB> = {
  O: [18, 24, 31],
  B: [102, 155, 210],
  S: [141, 180, 221],
  W: [235, 239, 244],
  K: [18, 24, 31],
  T: [156, 169, 183],
  G: [134, 203, 146],
  Y: [230, 191, 95],
  R: [232, 112, 112],
};

const BASE: readonly string[] = [
  "........................",
  "..........OOOOOO........",
  "........OOBBBBBO........",
  ".......OBBSSSWWBBO......",
  ".......OBSSSWKKWBBO.....",
  ".......OBBSSSWWBBBO.....",
  "........OBBBBBBBBO..OO..",
  ".......OOBWWBBBBO..OBO..",
  "...OOOOBBOBWWWBBBOOBBO..",
  "..OBBBBO.OBWWWBBBBBBBO..",
  "..OBBOOO.OBWWBBBBOOOO...",
  "..OBBO...OBWWBBBBO......",
  "...OBBBBOOBWWBBO........",
  "....OOOBBBBBBBOBO.......",
  ".......OBO..OBO.........",
  "......OOOO..OOOO........",
];

const TYPING: readonly Dot[] = [
  [13, 10, "B"], [14, 10, "B"], [15, 11, "B"],
  [15, 12, "O"], [16, 12, "O"], [17, 12, "O"], [18, 12, "O"],
  [19, 12, "O"], [20, 12, "O"], [21, 12, "O"], [22, 12, "O"],
  [14, 13, "O"], [15, 13, "T"], [16, 13, "T"], [17, 13, "T"],
  [18, 13, "T"], [19, 13, "T"], [20, 13, "T"], [21, 13, "T"],
  [22, 13, "T"], [23, 13, "O"],
  [14, 14, "O"], [15, 14, "O"], [16, 14, "O"], [17, 14, "O"],
  [18, 14, "O"], [19, 14, "O"], [20, 14, "O"], [21, 14, "O"],
  [22, 14, "O"], [23, 14, "O"],
];

const SUCCESS: readonly Dot[] = [
  [20, 0, "G"], [20, 1, "G"], [19, 1, "G"], [21, 1, "G"], [20, 2, "G"],
  [17, 3, "G"], [17, 4, "G"], [16, 4, "G"], [18, 4, "G"], [17, 5, "G"],
  [22, 5, "G"], [22, 6, "G"],
];

const WARNING: readonly Dot[] = [
  [20, 0, "Y"], [21, 0, "Y"], [20, 1, "Y"], [21, 1, "Y"],
  [20, 2, "Y"], [21, 2, "Y"], [20, 4, "Y"], [21, 4, "Y"],
];

const ERROR: readonly Dot[] = [
  [19, 0, "R"], [22, 0, "R"], [20, 1, "R"], [21, 1, "R"],
  [20, 2, "R"], [21, 2, "R"], [19, 3, "R"], [22, 3, "R"],
];

/** Derive one honest pose from controller activity and its latest outcome. */
export function mascotState(signal: MascotSignal): MascotState {
  if (signal.feedbackTone === "error" || signal.readinessTone === "error") return "error";
  if (signal.feedbackTone === "warn" || signal.readinessTone === "warn") return "warning";

  if (signal.activityKind !== undefined) {
    if (/waiting for you/i.test(signal.status ?? "")) return "warning";
    if (signal.activityKind === "command" || /\b(?:writing|running)\b/i.test(signal.status ?? "")) {
      return "typing";
    }
    return "thinking";
  }

  if (signal.feedbackTone === "info") return "success";

  const latest = signal.blocks.at(-1);
  if (latest?.kind === "answer") return "success";
  if (latest?.kind === "reasoning") return latest.live === true ? "thinking" : "idle";
  if (latest?.kind === "tool") {
    if (latest.tone === "fail") return "error";
    if (latest.tone === "deny") return "warning";
    return latest.tone === "pending" ? "typing" : "idle";
  }
  if (latest?.kind === "notice") {
    if (latest.tone === "error") return "error";
    if (latest.tone === "warn") return "warning";
  }
  return "idle";
}

/** Render a pose without writing to the terminal or depending on its background. */
export function renderMascot(
  state: MascotState,
  phase = 0,
  reducedMotion = false,
): string[] {
  const pixels = BASE.map((line) => [...line] as Pixel[]);
  const pulse = !reducedMotion && Math.floor(phase / 4) % 2 === 1;

  if (state === "thinking") {
    const shift = pulse ? 1 : 0;
    dots(pixels, [
      [19 + shift, 0, "O"], [20 + shift, 0, "W"], [21 + shift, 0, "O"],
      [19 + shift, 1, "O"], [20 + shift, 1, "W"], [21 + shift, 1, "O"],
      [17 + shift, 2, "O"], [18 + shift, 2, "W"],
      [17 + shift, 3, "O"], [18 + shift, 3, "O"],
    ]);
  } else if (state === "typing") {
    dots(pixels, TYPING);
    dots(pixels, [[pulse ? 20 : 17, 11, "W"]]);
  } else if (state === "success") {
    dots(pixels, SUCCESS);
  } else if (state === "warning") {
    dots(pixels, WARNING);
  } else if (state === "error") {
    dots(pixels, ERROR);
  }

  const rows: string[] = [];
  for (let y = 0; y < MASCOT_ROWS * 2; y += 2) {
    let line = "";
    for (let x = 0; x < MASCOT_COLS; x++) {
      line += cell(pixels[y]?.[x] ?? ".", pixels[y + 1]?.[x] ?? ".");
    }
    rows.push(line.replace(/ +$/, ""));
  }
  return rows;
}

function dots(pixels: Pixel[][], additions: readonly Dot[]): void {
  for (const [x, y, pixel] of additions) {
    if (x >= 0 && x < MASCOT_COLS && y >= 0 && y < MASCOT_ROWS * 2) {
      const row = pixels[y];
      if (row !== undefined) row[x] = pixel;
    }
  }
}

function cell(top: Pixel, bottom: Pixel): string {
  if (top === "." && bottom === ".") return " ";
  if (top === ".") return paint({ text: "▄", fg: COLORS[bottom as Exclude<Pixel, ".">] });
  if (bottom === ".") return paint({ text: "▀", fg: COLORS[top] });
  if (top === bottom) return paint({ text: "█", fg: COLORS[top] });
  return paint({ text: "▀", fg: COLORS[top], bg: COLORS[bottom] });
}
