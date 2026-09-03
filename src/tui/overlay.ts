// The modal interaction layer: a picker, a single-line field, or read-only help.

import type { Key } from "./keys.ts";
import type { Picker } from "./picker.ts";
import * as picker from "./picker.ts";
import type { Field } from "./field.ts";
import { oneLine } from "./field.ts";
import { applyKey } from "./input.ts";
import type { Modal } from "./modal.ts";
import { PromptLimitError } from "../input-boundary.ts";

export type Open =
  | { picker: Picker; settle(index?: number): void }
  | { field: Field; settle(text?: string): void }
  | { help: true; settle(): void };

export type Outcome = { open?: Open; abort?: boolean; quit?: boolean; inputLimit?: boolean };

export function shown(open: Open | undefined): Modal | undefined {
  if (open === undefined) return undefined;
  if ("picker" in open) return { kind: "pick", picker: open.picker };
  if ("field" in open) return { kind: "type", field: open.field };
  return { kind: "help" };
}

export function cancel(open: Open | undefined): undefined {
  if (open === undefined) return undefined;
  if ("help" in open) open.settle();
  else open.settle(undefined);
  return undefined;
}

export function handle(open: Open, key: Key): Outcome {
  if (key.ctrl && key.name === "c") {
    cancel(open);
    return { abort: true };
  }
  if (key.ctrl && key.name === "d") {
    cancel(open);
    return { quit: true };
  }
  if (key.name === "escape") {
    cancel(open);
    return {};
  }
  try {
    if ("picker" in open) return handlePicker(open, key);
    if ("field" in open) return handleField(open, key);
  } catch (error) {
    if (error instanceof PromptLimitError) return { open, inputLimit: true };
    throw error;
  }
  return { open };
}

function handlePicker(open: Extract<Open, { picker: Picker }>, key: Key): Outcome {
  switch (key.name) {
    case "up":
      open.picker = picker.move(open.picker, -1);
      break;
    case "down":
      open.picker = picker.move(open.picker, 1);
      break;
    case "left":
      open.picker = picker.adjust(open.picker, -1);
      break;
    case "right":
      open.picker = picker.adjust(open.picker, 1);
      break;
    case "home":
      open.picker = picker.edge(open.picker, "home");
      break;
    case "end":
      open.picker = picker.edge(open.picker, "end");
      break;
    case "pageup":
      open.picker = picker.page(open.picker, -1);
      break;
    case "pagedown":
      open.picker = picker.page(open.picker, 1);
      break;
    case "backspace":
      open.picker = picker.backspace(open.picker);
      break;
    case "enter":
      if (picker.selected(open.picker) === undefined) break;
      open.settle(open.picker.index);
      return {};
    case "paste":
      if (open.picker.searchable === true) {
        open.picker = picker.type(open.picker, key.text.replace(/\s+/g, " "));
      }
      break;
    case "char":
      if (open.picker.searchable === true) {
        open.picker = picker.type(open.picker, key.text);
        break;
      }
      {
        const index = picker.byKey(open.picker, key.text);
        if (index === undefined) break;
        open.settle(index);
        return {};
      }
    default:
      if (key.ctrl && key.name === "u" && open.picker.searchable === true) {
        open.picker = picker.clear(open.picker);
      }
  }
  return { open };
}

function handleField(open: Extract<Open, { field: Field }>, key: Key): Outcome {
  if (key.name === "enter") {
    const text = open.field.editor.text.trim();
    open.settle(text === "" ? undefined : text);
    return {};
  }

  const edited = applyKey(open.field.editor, key);
  if (edited !== undefined) open.field = { ...open.field, editor: oneLine(edited) };
  return { open };
}
