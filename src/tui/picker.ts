// One interaction model for every terminal selector.

import type { Palette } from "../ui/theme.ts";
import type { Seg } from "../ui/render.ts";
import { row } from "../ui/render.ts";
import { elide } from "../ui/width.ts";
import { menuWindow, renderMenuRows } from "./components/menu.ts";
import { promptCursor, promptLine } from "./components/prompt.ts";
import type { Cursor } from "./frame.ts";

export type Option = {
  label: string;
  description?: string;
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
  options: readonly Option[];
  searchable?: boolean;
  query?: string;
  /** Override the compact default when one control plane should show more rows. */
  visible?: number;
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
  return withQuery(picker, `${picker.query ?? ""}${text}`);
}

export function backspace(picker: Picker): Picker {
  if (picker.searchable !== true || (picker.query ?? "") === "") return picker;
  return withQuery(picker, Array.from(picker.query ?? "").slice(0, -1).join(""));
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
  maxRows = (picker.visible ?? WINDOW) + 3,
): string[] {
  return layout(picker, width, pal, maxRows).rows;
}

/** Caret for the shared query row, relative to the unframed picker body. */
export function caret(
  picker: Picker,
  width: number,
  maxRows = (picker.visible ?? WINDOW) + 3,
): Cursor | undefined {
  return layout(picker, width, undefined, maxRows).cursor;
}

function layout(
  picker: Picker,
  width: number,
  pal: Palette | undefined,
  maxRows: number,
): { rows: string[]; cursor?: Cursor } {
  const found = matches(picker);
  const note = picker.description ?? picker.footer;
  const hasTitle = picker.title.length > 0;
  const fixed = (hasTitle ? 1 : 0) +
    (note === undefined ? 0 : 1) +
    (picker.searchable === true ? 1 : 0);
  const optionRoom = Math.max(1, Math.min(picker.visible ?? WINDOW, maxRows - fixed));
  const selectedAt = Math.max(0, found.findIndex((entry) => entry.index === picker.index));
  const { first, last } = menuWindow(found.length, selectedAt, optionRoom);
  const shown = found.slice(first, last);
  const colors = pal;
  const options = colors === undefined
    ? []
    : shown.length === 0
    ? [row(width, [{ text: "no matches", fg: colors.ink.muted }])]
    : renderMenuRows(
        shown.map(({ option, index }) => ({
          label: option.label,
          description: option.description,
          hint: option.hint,
          value: option.value,
          adjustable: option.adjustable,
          selected: index === picker.index,
        })),
        width,
        colors,
      );

  const progress = found.length > shown.length || picker.searchable === true
    ? `${found.length === 0 ? "0" : `${first + 1}–${first + shown.length}`} / ${found.length}` +
      (found.length === picker.options.length ? "" : ` · ${picker.options.length} total`)
    : "";
  const titleRight = hasTitle
    ? picker.right ?? (picker.searchable === true ? undefined : progress)
    : undefined;
  const visibleTitleRight = titleRight === undefined
    ? undefined
    : elide(titleRight, Math.max(1, Math.floor(width * 0.55)));
  const queryRow = picker.searchable === true && colors !== undefined
    ? promptLine(picker.query ?? "", (picker.query ?? "").length, width, colors, {
        placeholder: "type to filter",
        right: progress,
      })
    : undefined;
  const queryCursor = picker.searchable === true
    ? promptCursor(picker.query ?? "", (picker.query ?? "").length, width, { right: progress })
    : undefined;
  const queryOffset = (hasTitle ? 1 : 0) + (note === undefined ? 0 : 1);

  return {
    rows: colors === undefined
      ? []
      : [
          ...(hasTitle
            ? [row(
                width,
                picker.title,
                visibleTitleRight === undefined || visibleTitleRight === ""
                  ? []
                  : [{ text: visibleTitleRight, fg: colors.ink.dim }],
              )]
            : []),
          ...(note === undefined ? [] : [row(width, [{ text: note, fg: colors.ink.muted }])]),
          ...(queryRow === undefined ? [] : [queryRow.row]),
          ...options,
        ],
    cursor: picker.searchable === true
      ? { row: queryOffset, col: queryCursor?.col ?? 0 }
      : undefined,
  };
}

export function heading(label: string, about: string, pal: Palette): Seg[] {
  return [
    { text: `${label}  `, fg: pal.accent, bold: true },
    { text: about, fg: pal.ink.fg },
  ];
}

type Match = { option: Option; index: number };

function matches(picker: Picker): Match[] {
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
