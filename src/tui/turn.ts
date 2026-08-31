// Turning what the controller reports into what the transcript shows.
//
// The controller speaks in stream events and tool results; the screen speaks in
// blocks. This is the whole of the translation, kept out of the shell so that
// neither has to know how the other is built.

import type { ControllerEvents } from "../controller.ts";
import type { ToolCallBlock, Usage } from "../types.ts";
import type { ToolPreview } from "../tools/index.ts";
import { condense, diff } from "../ui/diff.ts";
import type { Answer } from "./approve.ts";
import { promptFor } from "./approve.ts";
import type { Picker } from "./picker.ts";
import type { Block, Detail } from "./blocks.ts";
import type { Palette } from "../ui/theme.ts";
export type Stage = {
  emit(block: Block): void;
  render(block?: Block): void;
  /** Put a question up and call back once the user has picked an answer. */
  ask(prompt: Picker, settle: (answer: Answer) => void): void;
  /** Whether this call may run under the current session policy or a remembered grant. */
  approved(call: ToolCallBlock): boolean;
  /** Stop asking about this narrow call scope for the rest of the session. */
  remember(call: ToolCallBlock): void;
  /** Track whether a turn is active so the footer can offer interruption. */
  status(text: string): void;
  usage?(usage: Usage): void;
  palette: Palette;
};

export type Transcription = ControllerEvents & {
  /** Seal the final stream and settle any tool whose result was interrupted. */
  finish(reason?: "interrupted" | "failed"): void;
};

// Semantic activity labels remain useful state even though the quiet footer
// reduces them to one stable interruption hint. Reasoning and tools identify
// the live work in the transcript itself.
const WAITING = "Waiting";
const THINKING = "Thinking";
const WRITING = "Writing";
const ASKING = "Waiting for you";

/** Unchanged rows kept either side of a change. */
const CONTEXT = 2;

export function transcribe(stage: Stage): Transcription {
  // The block the stream is currently filling. A change of kind starts a new
  // one, which is what keeps reasoning and answer from running together.
  let open: { kind: "answer" | "reasoning"; block: Block } | undefined;
  const tools = new Map<string, Block>();
  let step = 1;
  let steps = 1;
  let tool = 1;
  let toolTotal = 1;

  const close = (): Block | undefined => {
    const block = open?.block;
    const changed = block?.kind === "reasoning" && block.live === true;
    if (changed && block.kind === "reasoning") block.live = false;
    open = undefined;
    return changed ? block : undefined;
  };

  return {
    finish(reason) {
      const changed = close();
      if (changed !== undefined) stage.render(changed);
      for (const block of tools.values()) {
        if (block.kind !== "tool" || block.tone !== "pending") continue;
        block.tone = "fail";
        block.right = reason ?? "failed";
        block.startedAt = undefined;
        stage.render(block);
      }
    },

    onStep(current, total) {
      step = current;
      steps = total;
      stage.status(waiting(step, steps));
      stage.render();
    },

    onToolProgress(current, total) {
      tool = current;
      toolTotal = total;
    },

    onUsage(usage) {
      stage.usage?.(usage);
    },

    onStatus(status) {
      stage.status(status);
      stage.render();
    },

    onStream(event) {
      const kind = event.kind === "thinking" ? "reasoning" : "answer";
      stage.status(kind === "reasoning" ? THINKING : WRITING);

      if (open === undefined || open.kind !== kind) {
        const changed = close();
        if (changed !== undefined) stage.render(changed);
        const block: Block = kind === "reasoning"
          ? { kind, text: "", live: true, expanded: false }
          : { kind, text: "" };
        open = { kind, block };
        stage.emit(block);
      }

      if (open.block.kind === "answer" || open.block.kind === "reasoning") {
        open.block.text += event.text;
      }
      stage.render(open.block);
    },

    onToolCall(call, look) {
      const changed = close();
      if (changed !== undefined) stage.render(changed);
      stage.status(
        `Running ${call.name}${toolTotal > 1 ? ` · tool ${tool}/${toolTotal}` : ""}${step > 1 ? ` · step ${step}/${steps}` : ""}`,
      );
      const block: Block = {
        kind: "tool",
        name: call.name,
        target: target(call.input),
        right: "running",
        tone: "pending",
        body: preview(call, look),
        startedAt: Date.now(),
      };
      tools.set(call.id, block);
      stage.emit(block);
      stage.render(block);
    },

    onToolOutput(call, output) {
      const block = tools.get(call.id);
      if (block === undefined || block.kind !== "tool" || block.tone !== "pending") return;
      block.body = details(output, "out");
      stage.render(block);
    },

    onToolResult(call, result, summary) {
      stage.status(waiting(step, steps));
      const block = tools.get(call.id);
      if (block === undefined || block.kind !== "tool") return;
      if (block.tone === "deny") {
        stage.render(block);
        return;
      }

      block.tone = result.isError ? "fail" : "ok";
      block.right = summary ?? "";
      block.startedAt = undefined;

      // A failure replaces the preview: what the call was going to do stops
      // being the interesting part the moment it did not do it.
      const outcome = result.isError
        ? details(result.output, "out")
        : produced(call, result.output);
      if (outcome !== undefined) block.body = outcome;

      stage.render(block);
    },

    approve(call) {
      const block = tools.get(call.id);
      if (stage.approved(call)) {
        if (block !== undefined && block.kind === "tool") {
          block.right = "running";
          block.startedAt ??= Date.now();
          stage.render(block);
        }
        return Promise.resolve(true);
      }
      const changed = close();
      if (changed !== undefined) stage.render(changed);
      stage.status(ASKING);
      if (block !== undefined && block.kind === "tool") {
        block.right = pendingApproval(block.body);
        block.startedAt = undefined;
        stage.render(block);
      }

      return new Promise<boolean>((resolve) => {
        stage.ask(promptFor(call, target(call.input), stage.palette), (answer) => {
          if (answer === "always") stage.remember(call);
          const approved = answer !== "no";

          const settled = tools.get(call.id);
          if (settled !== undefined && settled.kind === "tool") {
            if (approved) {
              settled.tone = "pending";
              settled.right = "running";
              settled.startedAt = Date.now();
            } else {
              settled.tone = "deny";
              settled.right = "denied";
              settled.startedAt = undefined;
            }
          }
          stage.status(approved ? `Running ${call.name}` : waiting(step, steps));
          stage.render(settled);
          resolve(approved);
        });
      });
    },
  };
}

function pendingApproval(body: Detail[] | undefined): string {
  const added = body?.filter((detail) => detail.kind === "add").length ?? 0;
  const removed = body?.filter((detail) => detail.kind === "del").length ?? 0;
  const summary = added + removed === 0 ? "" : `+${added} −${removed} · `;
  return `${summary}pending approval`;
}

function waiting(step: number, total: number): string {
  return step === 1 ? WAITING : `${WAITING} · step ${step}/${total}`;
}

/** The argument worth showing: the thing the call acts on. */
function target(input: Record<string, unknown>): string {
  const { path, command } = input as { path?: unknown; command?: unknown };
  if (typeof command === "string") return command;
  if (typeof path === "string") return path;
  const rest = JSON.stringify(input);
  return rest === "{}" ? "" : rest;
}

/**
 * What the call is about to do, drawn before it is allowed to do it.
 *
 * The tool is asked first, because only it knows what is already on disk: a
 * write against an existing file is a replacement, and showing it as a page of
 * additions hides exactly the part worth approving. The fallback diffs what is
 * in the arguments, which is all there is when a tool has nothing to say.
 */
function preview(call: ToolCallBlock, look?: ToolPreview): Detail[] | undefined {
  if (look !== undefined) return changes(look.before, look.after);

  const input = call.input as { content?: unknown; old_text?: unknown; new_text?: unknown };
  if (call.name === "write_file" && typeof input.content === "string") {
    return changes("", input.content);
  }
  if (call.name === "edit_file" && typeof input.old_text === "string") {
    return changes(input.old_text, typeof input.new_text === "string" ? input.new_text : "");
  }

  return undefined;
}

/** Two texts as the rows of their difference, unchanged runs summed up. */
function changes(before: string, after: string): Detail[] | undefined {
  let oldLine = 1;
  let newLine = 1;
  const rows: Detail[] = [];

  for (const changed of condense(diff(before, after), CONTEXT)) {
    if (changed.kind === "gap") {
      rows.push({ kind: "gap", text: `… ${changed.skipped} unchanged` });
      oldLine += changed.skipped;
      newLine += changed.skipped;
      continue;
    }
    if (changed.kind === "keep") {
      rows.push({ kind: "keep", text: tabs(changed.text), oldLine, newLine });
      oldLine++;
      newLine++;
      continue;
    }
    if (changed.kind === "del") {
      rows.push({ kind: "del", text: tabs(changed.text), oldLine });
      oldLine++;
      continue;
    }
    rows.push({ kind: "add", text: tabs(changed.text), newLine });
    newLine++;
  }

  emphasizePairs(rows);
  return rows.length === 0 ? undefined : rows;
}

function emphasizePairs(rows: Detail[]): void {
  for (let index = 0; index < rows.length - 1; index++) {
    const removed = rows[index];
    const added = rows[index + 1];
    if (removed?.kind !== "del" || added?.kind !== "add") continue;
    if (rows[index - 1]?.kind === "del" || rows[index + 2]?.kind === "add") continue;

    let start = 0;
    while (start < removed.text.length && start < added.text.length && removed.text[start] === added.text[start]) start++;
    let suffix = 0;
    while (
      suffix < removed.text.length - start &&
      suffix < added.text.length - start &&
      removed.text[removed.text.length - 1 - suffix] === added.text[added.text.length - 1 - suffix]
    ) suffix++;
    while (suffix > 0 && (!wordBoundary(removed.text, suffix) || !wordBoundary(added.text, suffix))) suffix--;

    const removedLength = removed.text.length - start - suffix;
    const addedLength = added.text.length - start - suffix;
    if (removedLength > 0) removed.emphasis = { start, length: removedLength };
    if (addedLength > 0) added.emphasis = { start, length: addedLength };
  }
}

function wordBoundary(text: string, suffix: number): boolean {
  const at = text.length - suffix;
  if (at <= 0 || at >= text.length) return true;
  return /\w/.test(text[at - 1] as string) !== /\w/.test(text[at] as string);
}

/** What the call left behind, for the calls whose output is worth a look. */
function produced(call: ToolCallBlock, output: string): Detail[] | undefined {
  if (call.name === "run_command") return details(output, "out");
  // A read or a listing is already summed up on the right of its own row, and
  // the model is about to say what was in it. Printing it twice helps nobody.
  return undefined;
}

function details(text: string, kind: "out"): Detail[] | undefined {
  const rows = split(text).map((line) => ({ kind, text: tabs(line) }));
  return rows.length === 0 ? undefined : rows;
}

function split(text: string): string[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map(tabs);
}

// A tab is a width the terminal decides and the row measurement cannot see.
function tabs(line: string): string {
  return line.replace(/\t/g, "  ");
}
