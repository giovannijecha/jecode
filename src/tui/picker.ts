// One interaction model for every terminal selector.

import type { Palette } from "../ui/theme.ts";
import type { Seg } from "../ui/render.ts";
import { graphemes } from "../ui/width.ts";
import { assertPromptAppend } from "../input-boundary.ts";
import { layoutPicker } from "./picker-layout.ts";
import type { Cursor } from "./frame.ts";

export type Option = {
  label: string;
  hint?: string;
  /** A compact row-local value which remains visible on narrow terminals. */
  value?: string;
  /** Show the selected value as a left/right stepper. */
  adjustable?: boolean;
  key?: string;
};

export type Picker = {
  title: Seg[];
  description?: string;
  right?: string;
  footer?: string;
  /** Explicit controls for interactions whose Esc action differs from closing. */
  controls?: string;
  options: readonly Option[];
  searchable?: boolean;
  query?: string;
  /** Override the compact default when one control plane should show more rows. */
  visible?: number;
  /** Repeat clipped choice content below the list by default, or only elide it. */
  overflow?: "detail" | "truncate";
  /** Change the selected row without settling or closing the picker. */
  adjust?(index: number, step: -1 | 1): Picker;
  index: number;
};

const WINDOW = 6;

export function move(picker: Picker, step: number): Picker {
  const shown = matches(picker);
  if (shown.length === 0) return picker;
  const at = Math.max(0, shown.findIndex((entry) => entry.index === picker.index));
  return { ...picker, index: (shown[((at + step) % shown.length + shown.length) % shown.length] as Match).index };
}

export function edge(picker: Picker, end: "home" | "end"): Picker {
  const shown = matches(picker);
  const found = end === "home" ? shown[0] : shown.at(-1);
  return found === undefined ? picker : { ...picker, index: found.index };
}

export function page(picker: Picker, direction: -1 | 1, rows = WINDOW): Picker {
  return move(picker, direction * Math.max(1, rows));
}

export function adjust(picker: Picker, step: -1 | 1): Picker {
  const index = selected(picker);
  return index === undefined || picker.adjust === undefined
    ? picker
    : picker.adjust(index, step);
}

export function type(picker: Picker, text: string): Picker {
  if (picker.searchable !== true || text === "") return picker;
  assertPromptAppend((picker.query ?? "").length, text.length);
  return withQuery(picker, `${picker.query ?? ""}${text}`);
}

export function backspace(picker: Picker): Picker {
  if (picker.searchable !== true || (picker.query ?? "") === "") return picker;
  return withQuery(picker, graphemes(picker.query ?? "").slice(0, -1).join(""));
}

export function clear(picker: Picker): Picker {
  return withQuery(picker, "");
}

export function selected(picker: Picker): number | undefined {
  return matches(picker).some((entry) => entry.index === picker.index) ? picker.index : undefined;
}

export function byKey(picker: Picker, text: string): number | undefined {
  const typed = text.trim().toLowerCase();
  if (typed === "") return undefined;
  const digit = Number(typed);
  if (Number.isInteger(digit) && digit >= 1 && digit <= picker.options.length) return digit - 1;
  const found = picker.options.findIndex((option) => option.key === typed);
  return found === -1 ? undefined : found;
}

export function panel(
  picker: Picker,
  width: number,
  pal: Palette,
  maxRows = (picker.visible ?? WINDOW) + 6,
): string[] {
  return layoutPicker(picker, matches(picker), width, pal, maxRows).rows;
}

/** Caret for the shared query row, relative to the unframed picker body. */
export function caret(
  picker: Picker,
  width: number,
  maxRows = (picker.visible ?? WINDOW) + 6,
): Cursor | undefined {
  return layoutPicker(picker, matches(picker), width, undefined, maxRows).cursor;
}

export function heading(label: string, about: string, pal: Palette): Seg[] {
  return [
    { text: `${label}  `, fg: pal.accent, bold: true },
    { text: about, fg: pal.ink.fg },
  ];
}

export type Match = { option: Option; index: number };

export function matches(picker: Picker): Match[] {
  const query = (picker.query ?? "").trim().toLocaleLowerCase();
  return picker.options.flatMap((option, index) => {
    const haystack = `${option.label} ${option.hint ?? ""} ${option.value ?? ""}`.toLocaleLowerCase();
    return query === "" || haystack.includes(query) ? [{ option, index }] : [];
  });
}

function withQuery(picker: Picker, query: string): Picker {
  const next = { ...picker, query };
  return { ...next, index: matches(next)[0]?.index ?? 0 };
}
