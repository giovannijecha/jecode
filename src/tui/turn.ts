// Turning what the controller reports into what the transcript shows.
//
// The controller speaks in stream events and tool results; the screen speaks in
// blocks. This is the whole of the translation, kept out of the shell so that
// neither has to know how the other is built.

import type { ControllerEvents } from "../controller.ts";
import type { ToolCallBlock, Usage } from "../types.ts";
import type { Answer } from "./approve.ts";
import { promptFor } from "./approve.ts";
import type { Picker } from "./picker.ts";
import type { Block, Detail } from "./blocks.ts";
import type { Palette } from "../ui/theme.ts";
import { toolTarget, previewDetails, producedDetails, outputDetails } from "./tool-details.ts";

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
  requestInput?(inputTokens: number): void;
  palette: Palette;
};

export type Transcription = ControllerEvents & {
  /** Seal the final stream and settle any tool whose result was interrupted. */
  finish(reason?: "interrupted" | "failed"): void;
};

// Semantic activity labels feed the footer's compact state and timer while
// reasoning and tools keep the detailed work visible in the transcript.
const WAITING = "Waiting";
const THINKING = "Thinking";
const RESPONDING = "Responding";
const ASKING = "Waiting for you";

export function transcribe(stage: Stage): Transcription {
  // The block the stream is currently filling. A change of kind starts a new
  // one, which is what keeps reasoning and answer from running together.
  let open: { kind: "answer" | "reasoning"; block: Block } | undefined;
  const tools = new Map<string, Block>();
  const progress = new Map<string, { current: number; total: number }>();

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
        settleDuration(block);
        stage.render(block);
      }
    },

    onToolPreparing(call, current, total) {
      progress.set(call.id, { current, total });
      const changed = close();
      if (changed !== undefined) stage.render(changed);
      stage.status(toolStatus("Preparing", call, progress));
      stage.render();
    },

    onToolStart(call, current, total) {
      progress.set(call.id, { current, total });
      stage.status(toolStatus("Running", call, progress));
      const block = tools.get(call.id);
      if (block !== undefined && block.kind === "tool") {
        block.right = "running";
        block.startedAt = Date.now();
        block.durationMs = undefined;
      }
      stage.render(block);
    },

    onUsage(usage) {
      stage.usage?.(usage);
    },

    onRequestInput(inputTokens) {
      stage.requestInput?.(inputTokens);
    },

    onStatus(status) {
      stage.status(status);
      stage.render();
    },

    onStream(event) {
      if (event.kind === "tool") {
        const changed = close();
        if (changed !== undefined) stage.render(changed);
        stage.status(`Preparing ${event.name ?? "tool"}`);
        stage.render();
        return;
      }
      const kind = event.kind === "thinking" ? "reasoning" : "answer";
      stage.status(kind === "reasoning" ? THINKING : RESPONDING);

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
      const block: Block = {
        kind: "tool",
        name: call.name,
        target: toolTarget(call.input),
        right: "ready",
        tone: "pending",
        body: previewDetails(call, look),
      };
      tools.set(call.id, block);
      stage.emit(block);
      stage.render(block);
    },

    onToolOutput(call, output) {
      const block = tools.get(call.id);
      if (block === undefined || block.kind !== "tool" || block.tone !== "pending") return;
      block.body = outputDetails(output, "out");
      stage.render(block);
    },

    onToolResult(call, result, summary) {
      stage.status(WAITING);
      const block = tools.get(call.id);
      if (block === undefined || block.kind !== "tool") return;
      // Explicit refusal stays a refusal. Cancellation can first settle an
      // approval overlay as `no`, though, so its synthesized result must be
      // allowed to reconcile that rail with the interrupted history.
      if (block.tone === "deny" && summary !== "interrupted") {
        stage.render(block);
        return;
      }

      block.tone = result.isError ? "fail" : "ok";
      block.right = summary ?? "";
      settleDuration(block);

      // A failure replaces the preview: what the call was going to do stops
      // being the interesting part the moment it did not do it.
      const outcome = result.isError
        ? outputDetails(result.output, "out")
        : producedDetails(call, result.output);
      if (outcome !== undefined) block.body = outcome;

      stage.render(block);
    },

    approve(call) {
      const block = tools.get(call.id);
      if (stage.approved(call)) return Promise.resolve(true);
      const changed = close();
      if (changed !== undefined) stage.render(changed);
      stage.status(ASKING);
      if (block !== undefined && block.kind === "tool") {
        block.right = pendingApproval(block.body);
        block.startedAt = undefined;
        stage.render(block);
      }

      return new Promise<boolean>((resolve) => {
        stage.ask(promptFor(call, toolTarget(call.input), stage.palette), (answer) => {
          if (answer === "always") stage.remember(call);
          const approved = answer !== "no";

          const settled = tools.get(call.id);
          if (settled !== undefined && settled.kind === "tool") {
            if (approved) {
              settled.tone = "pending";
              settled.right = "ready";
              settled.startedAt = undefined;
            } else {
              settled.tone = "deny";
              settled.right = "denied";
              settled.startedAt = undefined;
            }
          }
          stage.status(
            approved
              ? toolStatus("Preparing", call, progress)
              : WAITING,
          );
          stage.render(settled);
          resolve(approved);
        });
      });
    },
  };
}

function toolStatus(
  phase: "Preparing" | "Running",
  call: ToolCallBlock,
  progress: ReadonlyMap<string, { current: number; total: number }>,
): string {
  const position = progress.get(call.id);
  const tool = position !== undefined && position.total > 1
    ? ` · tool ${position.current}/${position.total}`
    : "";
  return `${phase} ${call.name}${tool}`;
}

function pendingApproval(body: Detail[] | undefined): string {
  const added = body?.filter((detail) => detail.kind === "add").length ?? 0;
  const removed = body?.filter((detail) => detail.kind === "del").length ?? 0;
  const summary = added + removed === 0 ? "" : `+${added} −${removed} · `;
  return `${summary}pending approval`;
}

function settleDuration(block: Extract<Block, { kind: "tool" }>): void {
  if (block.startedAt !== undefined) {
    block.durationMs = Math.max(0, Date.now() - block.startedAt);
  }
  block.startedAt = undefined;
}
