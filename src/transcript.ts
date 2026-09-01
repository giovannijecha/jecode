// A portable transcript: screen blocks in, Markdown out.

import type { Block, Detail } from "./tui/blocks.ts";
import { terminalText } from "./ui/terminal-text.ts";

export function defaultTranscriptName(now = new Date()): string {
  return `jecode-transcript-${now.toISOString().replace(/[-:.]/g, "")}.md`;
}

export function transcriptMarkdown(blocks: readonly Block[]): string {
  const out: string[] = ["# jecode transcript", ""];

  for (const block of blocks) {
    switch (block.kind) {
      case "user":
        out.push("## You", "", safeMultiline(block.text), "");
        break;
      case "answer":
        out.push("## Assistant", "", safeMultiline(block.text), "");
        break;
      case "reasoning":
        out.push(
          "<details>",
          "<summary>Reasoning</summary>",
          "",
          safeMultiline(block.text),
          "",
          "</details>",
          "",
        );
        break;
      case "tool":
        out.push(
          `- **${safeInline(block.name)}**${block.target === "" ? "" : ` \`${inlineCode(block.target)}\``}${block.right === "" ? "" : ` — ${safeInline(block.right)}`}`,
        );
        if ((block.body?.length ?? 0) > 0) {
          out.push("", "````text", ...(block.body ?? []).map(detail), "````", "");
        }
        break;
      case "notice":
        out.push(`> ${block.tone.toUpperCase()}: ${safeMultiline(block.text).replaceAll("\n", "\n> ")}`, "");
        break;
    }
  }

  while (out[out.length - 1] === "") out.pop();
  return `${out.join("\n")}\n`;
}

function detail(row: Detail): string {
  const prefix = row.kind === "add" ? "+ " : row.kind === "del" ? "- " : row.kind === "keep" ? "  " : "";
  return `${prefix}${safeInline(row.text)}`;
}

function inlineCode(text: string): string {
  return safeInline(text).replace(/`/g, "ˋ");
}

function safeInline(text: string): string {
  return terminalText(text);
}

function safeMultiline(text: string): string {
  return terminalText(text, { multiline: true });
}
