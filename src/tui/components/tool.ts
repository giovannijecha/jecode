import type { Palette, RGB } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { blank, row } from "../../ui/render.ts";
import type { Detail, Emphasis, ToolBlock, ToolTone } from "./types.ts";

const PAD = 1;
const PREVIEW_ROWS = 10;

export function renderTool(block: ToolBlock, width: number, pal: Palette): string[] {
  const ground = toolGround(block.tone, pal);
  const all = block.body ?? [];
  const shown = block.expanded === true ? all : all.slice(0, PREVIEW_ROWS);
  const remaining = all.length - shown.length;
  const rows = [
    "",
    blank(width, ground),
    row(
      width,
      [
        { text: block.name, fg: pal.ink.bright, bold: true },
        ...(block.target === "" ? [] : [{ text: ` ${block.target}`, fg: pal.accent }]),
      ],
      [],
      ground,
      PAD,
    ),
  ];

  if (shown.length > 0) {
    rows.push(blank(width, ground));
    rows.push(...shown.map((detail) => renderDetail(detail, block.tone, width, ground, pal)));
  }
  if (remaining > 0) {
    rows.push(
      row(
        width,
        [{ text: `… ${remaining} more lines · ctrl+o expand`, fg: pal.ink.muted }],
        [],
        ground,
        PAD,
      ),
    );
  }
  if (block.right !== "") {
    rows.push(
      row(width, [{ text: resultLabel(block.right, block.tone), fg: statusInk(block.tone, pal) }], [], ground, PAD),
    );
  }
  rows.push(blank(width, ground));
  return rows;
}

function renderDetail(
  detail: Detail,
  tone: ToolTone,
  width: number,
  ground: RGB,
  pal: Palette,
): string {
  if (detail.kind === "out") {
    const fg = tone === "fail" || failureLine(detail.text) ? pal.ink.removed : pal.ink.muted;
    return row(width, [{ text: detail.text === "" ? " " : detail.text, fg }], [], ground, PAD);
  }
  if (detail.kind === "gap") {
    return row(width, [{ text: `     ${detail.text}`, fg: pal.ink.muted }], [], ground, PAD);
  }

  const number = detail.kind === "add" ? detail.newLine : detail.oldLine;
  const prefix = `${detail.kind === "add" ? "+" : detail.kind === "del" ? "-" : " "}${String(number ?? "").padStart(3)} `;
  const fg = detail.kind === "add"
    ? pal.ink.added
    : detail.kind === "del"
      ? pal.ink.removed
      : pal.ink.muted;
  return row(
    width,
    [{ text: prefix, fg }, ...emphasized(detail.text, detail.emphasis, fg)],
    [],
    ground,
    PAD,
  );
}

function emphasized(text: string, emphasis: Emphasis | undefined, fg: RGB): Seg[] {
  if (emphasis === undefined || emphasis.length <= 0) return [{ text, fg }];
  const start = Math.max(0, Math.min(text.length, emphasis.start));
  const end = Math.max(start, Math.min(text.length, start + emphasis.length));
  return [
    ...(start === 0 ? [] : [{ text: text.slice(0, start), fg }]),
    { text: text.slice(start, end), fg, inverse: true },
    ...(end === text.length ? [] : [{ text: text.slice(end), fg }]),
  ];
}

function toolGround(tone: ToolTone, pal: Palette): RGB {
  if (tone === "ok") return pal.surface.added;
  if (tone === "fail") return pal.surface.removed;
  if (tone === "deny") return pal.surface.attention;
  return pal.surface.inset;
}

function statusInk(tone: ToolTone, pal: Palette): RGB {
  if (tone === "fail") return pal.ink.removed;
  if (tone === "pending") return pal.ink.attention;
  return pal.ink.muted;
}

function failureLine(line: string): boolean {
  return line.startsWith("✖") || line.includes("AssertionError") || /\bfailed\b/i.test(line);
}

function resultLabel(result: string, tone: ToolTone): string {
  if (tone === "pending") return result;
  const match = /^(.*?)(?:\s*·\s*)?(\d+(?:\.\d+)?(?:ms|s))$/.exec(result);
  if (match === null) return result;
  const prefix = (match[1] ?? "").trim();
  return `${prefix === "" ? "" : `${prefix} · `}Took ${match[2]}`;
}
