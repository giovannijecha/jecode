// The permission question: what is being asked, and the answers on offer.
//
// A permission prompt is the one moment the agent hands control back, so it is
// worth more than a line of prose and a key to guess at. It is a menu — the
// generic one — with an attention title and three answers written out.
// Nothing is approved by a key nobody meant to press.

import type { Palette } from "../ui/theme.ts";
import type { ToolCallBlock } from "../types.ts";
import { scopeFor } from "../permissions.ts";
import type { PermissionScope } from "../permissions.ts";
import type { Option, Picker } from "./picker.ts";

export { scopeFor };

/** What the user decided. `always` also stops the question coming back. */
export type Answer = "once" | "always" | "no";

/**
 * The three answers, in the order a person actually wants them.
 *
 * "Always" is scoped to the tool and to this session: it is a way to stop being
 * asked about `write_file` twenty times in a row, not a setting that outlives
 * the window it was granted in.
 */
export function promptFor(call: ToolCallBlock, target: string, pal: Palette): Picker {
  const scope = scopeFor(call);
  const options: Option[] = [
    { label: "Yes, once", hint: "y", key: "y", description: "Approve only this call. Ask again next time." },
    { label: `Yes, ${scopeNoun(scope)} for the session`, hint: "a", key: "a",
      description: `Reuse approval for ${scopeNoun(scope)} in this session.` },
    { label: "No, and say why", hint: "n", key: "n", description: "Do not run this call. Return feedback to the model." },
  ];

  return {
    title: [{ text: question(call.name), fg: pal.ink.attention, bold: true }],
    right: `${call.name}${target === "" ? "" : ` · ${target}`}`,
    controls: "↑↓ choose · enter confirm · esc deny",
    options,
    index: 0,
  };
}

function question(name: string): string {
  if (name === "edit_file") return "Allow this edit?";
  if (name === "write_file") return "Write this file?";
  if (name === "run_command") return "Run this command?";
  return "Allow this call?";
}

function scopeNoun(scope: PermissionScope): string {
  if (scope.key.startsWith("file\0")) return "this file";
  if (scope.key.startsWith("command\0")) return "this command";
  return "this call";
}

const ANSWERS: readonly Answer[] = ["once", "always", "no"];

/** What picking row `index` meant. Anything unrecognised refuses. */
export function answerAt(index: number | undefined): Answer {
  return index === undefined ? "no" : (ANSWERS[index] ?? "no");
}
