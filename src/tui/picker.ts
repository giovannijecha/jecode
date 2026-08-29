// One interaction model for every terminal selector.

import type { Palette } from "../ui/theme.ts";
import type { Seg } from "../ui/render.ts";
import { row } from "../ui/render.ts";
import { elide } from "../ui/width.ts";
import { menuWindow, renderMenuRows } from "./components/menu.ts";

export type Option = {
  label: string;
  hint?: string;
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

export function panel(picker: Picker, width: number, pal: Palette, maxRows = WINDOW + 4): string[] {
  const found = matches(picker);
  const fixed = 2 + (picker.description === undefined ? 0 : 1) + (picker.searchable === true ? 1 : 0);
  const optionRoom = Math.max(1, Math.min(WINDOW, maxRows - fixed));
  const selectedAt = Math.max(0, found.findIndex((entry) => entry.index === picker.index));
  const { first, last } = menuWindow(found.length, selectedAt, optionRoom);
  const shown = found.slice(first, last);
  const options = shown.length === 0
    ? [row(width, [{ text: "  no matches", fg: pal.ink.muted }])]
    : renderMenuRows(
        shown.map(({ option, index }) => ({
          label: option.label,
          hint: option.hint,
          selected: index === picker.index,
        })),
        width,
        pal,
      );

  const progress = found.length > shown.length || picker.searchable === true
    ? `${found.length === 0 ? "0" : `${first + 1}–${first + shown.length}`} / ${found.length}` +
      (found.length === picker.options.length ? "" : ` · ${picker.options.length} total`) + " · "
    : "";
  const footer = picker.footer ?? "Enter to select · Esc to close";

  return [
    row(
      width,
      picker.title,
      picker.right === undefined ? [] : [{ text: picker.right, fg: pal.ink.muted }],
    ),
    ...(picker.description === undefined
      ? []
      : [row(width, [{ text: picker.description, fg: pal.ink.muted }])]),
    ...(picker.searchable === true
      ? [
          row(width, [
            { text: "  filter  ", fg: pal.ink.muted },
            { text: picker.query === "" || picker.query === undefined ? "type to search" : picker.query, fg: pal.ink.bright },
          ]),
        ]
      : []),
    ...options,
    row(width, [{ text: `  ${elide(progress + footer, Math.max(1, width - 2))}`, fg: pal.ink.muted }]),
  ];
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
    const haystack = `${option.label} ${option.hint ?? ""}`.toLocaleLowerCase();
    return query === "" || haystack.includes(query) ? [{ option, index }] : [];
  });
}

function withQuery(picker: Picker, query: string): Picker {
  const next = { ...picker, query };
  return { ...next, index: matches(next)[0]?.index ?? 0 };
}
