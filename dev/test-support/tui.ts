import * as edit from "../../src/tui/editor.ts";
import type { Block } from "../../src/tui/blocks.ts";
import { transcribe } from "../../src/tui/turn.ts";
import { STEEL } from "../../src/ui/theme.ts";

export function base() {
  return {
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    pal: STEEL,
    footer: {
      workspace: "~/Codex/jecode",
      provider: "Anthropic API",
      model: "claude-sonnet-5",
      effort: "high",
    },
  };
}

export function stage(log: string[]) {
  const blocks: Block[] = [];
  const events = transcribe({
    emit: (block) => blocks.push(block),
    render: () => {},
    ask: (_prompt, settle) => settle("once"),
    approved: () => true,
    remember: () => {},
    status: (text) => log.push(text),
    palette: STEEL,
  });
  return { blocks, events };
}

export const callOf = (id: string, name: string, input: Record<string, unknown>) =>
  ({ kind: "tool_call", id, name, input }) as const;

export function strip(rows: readonly string[]): string[] {
  const ESCAPE = String.fromCharCode(27);
  return rows.map((r) => r.replace(new RegExp(`${ESCAPE}\[[0-9;]*m`, "g"), ""));
}
