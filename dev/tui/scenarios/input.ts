import type { View } from "../../../src/tui/view.ts";
import type { LabState } from "../model.ts";
import * as edit from "../../../src/tui/editor.ts";
import { base } from "./shared.ts";
import { matches } from "../../../src/tui/complete.ts";
import type { Picker } from "../../../src/tui/picker.ts";
import { move } from "../../../src/tui/picker.ts";
import { modelChoices, modelQuery } from "../fixtures.ts";

export function commandsScene(state: LabState): View {
  const editor = edit.of("/");
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Type `/` to discover commands. The list narrows as you type." }],
    editor,
    scroll: 0,
    menu: matches(editor.text),
    menuIndex: state.selected,
  };
}

export function searchScene(state: LabState): View {
  let picker: Picker = {
    title: [],
    options: modelChoices,
    searchable: true,
    query: modelQuery,
    index: 0,
  };
  picker = move(picker, state.selected);
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Long pickers use the same writable prompt as every other dock input." }],
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "pick", picker },
  };
}

export function helpScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Operational reference stays outside the conversation." }],
    editor: edit.EMPTY,
    scroll: 0,
    modal: { kind: "help" },
  };
}

export function fieldScene(state: LabState): View {
  return {
    ...base(state),
    blocks: [{ kind: "answer", text: "Credentials stay outside the workspace and transcript." }],
    editor: edit.EMPTY,
    scroll: 0,
    modal: {
      kind: "type",
      field: {
        title: [
          { text: "paste key", fg: state.palette.ink.attention, bold: true },
          { text: "  OLLAMA_API_KEY", fg: state.palette.ink.fg },
        ],
        right: "enter ok · esc skip",
        editor: edit.of("masked-demo-value"),
        secret: true,
        note: "Environment values take precedence.",
      },
    },
  };
}
