// The permission question: what is being asked, and the answers on offer.
//
// A permission prompt is the one moment the agent hands control back, so it is
// worth more than a line of prose and a key to guess at. It is a menu — the
// generic one — with an attention title and three answers written out.
// Nothing is approved by a key nobody meant to press.

import type { Palette } from "../ui/theme.ts";
import type { ToolCallBlock } from "../types.ts";
import type { Option, Picker } from "./picker.ts";

/** What the user decided. `always` also stops the question coming back. */
export type Answer = "once" | "always" | "no";

/**
 * The three answers, in the order a person actually wants them.
 *
 * "Always" is scoped to the tool and to this session: it is a way to stop being
 * asked about `write_file` twenty times in a row, not a setting that outlives
 * the window it was granted in.
 */
export type PermissionScope = { key: string; label: string; summary: string };

/**
 * The narrow permission represented by "always".
 *
 * File-changing tools share a grant for one displayed path. Shell grants are
 * tied to the exact command line. Unknown dangerous tools fall back to their
 * exact, stable input rather than silently inheriting a tool-wide grant.
 */
export function scopeFor(call: ToolCallBlock): PermissionScope {
  const path = typeof call.input.path === "string" ? call.input.path : undefined;
  if ((call.name === "write_file" || call.name === "edit_file") && path !== undefined) {
    return { key: `file\0${path}`, label: `changes to ${path}`, summary: `file changes · ${path}` };
  }

  const command = typeof call.input.command === "string" ? call.input.command : undefined;
  if (call.name === "run_command" && command !== undefined) {
    return { key: `command\0${command}`, label: "this exact command", summary: `command · ${command}` };
  }

  return {
    key: `${call.name}\0${stable(call.input)}`,
    label: "this exact call",
    summary: `${call.name} · ${target(call.input)}`,
  };
}

export function promptFor(call: ToolCallBlock, target: string, pal: Palette): Picker {
  const scope = scopeFor(call);
  const options: Option[] = [
    { label: "Yes, once", hint: "enter", key: "y" },
    { label: `Yes, ${scopeNoun(scope)} for the session`, hint: "a", key: "a" },
    { label: "No, and say why", hint: "esc", key: "n" },
  ];

  return {
    title: [{ text: question(call.name), fg: pal.ink.attention, bold: true }],
    right: `${call.name}${target === "" ? "" : ` · ${target}`}`,
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

function stable(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, item]) => `${JSON.stringify(key)}:${stable(item)}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function target(input: Record<string, unknown>): string {
  const value = input.path ?? input.command;
  return typeof value === "string" ? value : stable(input);
}

const ANSWERS: readonly Answer[] = ["once", "always", "no"];

/** What picking row `index` meant. Anything unrecognised refuses. */
export function answerAt(index: number | undefined): Answer {
  return index === undefined ? "no" : (ANSWERS[index] ?? "no");
}
