// The non-interactive path: stdin is a pipe, so there is no screen to own.
//
// It exists so the agent can be scripted and tested. It shares the controller,
// the tools and the block renderer with the TUI — only the surface differs.

import * as readline from "node:readline/promises";
import { stdin, stdout } from "node:process";
import type { Message } from "./types.ts";
import type { Session } from "./session.ts";
import type { ContextRequest, ControllerEvents } from "./controller.ts";
import { runTurn } from "./controller.ts";
import { resolveContextPolicy } from "./context/capacity.ts";
import { compactContext } from "./context/compactor.ts";
import { compactSession } from "./context/manual.ts";
import type { ContextAnchor } from "./context/projection.ts";
import type { ContextPolicy } from "./context/policy.ts";
import { isContextOverflow } from "./context/policy.ts";
import { handleCommand } from "./commands.ts";
import type { Block } from "./tui/blocks.ts";
import { renderBatch } from "./batch-view.ts";
import { columns } from "./ui/render.ts";
import { terminalText } from "./ui/terminal-text.ts";
import { recordAuxiliaryUsage, recordRequestInput, recordUsage } from "./usage.ts";

export type BatchEnvironment = {
  lines?: AsyncIterable<string>;
  write?(text: string): void;
  width?: number;
  signal?: AbortSignal;
};

export async function runBatch(session: Session, environment: BatchEnvironment = {}): Promise<void> {
  const rl = environment.lines === undefined ? readline.createInterface({ input: stdin }) : undefined;
  const lines = environment.lines ?? (rl as AsyncIterable<string>);
  const write = environment.write ?? ((text: string) => stdout.write(text));
  const width = environment.width ?? columns();
  const signal = environment.signal;
  const stopInput = (): void => rl?.close();
  signal?.addEventListener("abort", stopInput, { once: true });

  const emit = (block: Block): void => {
    for (const line of renderBatch(block, width, session.palette)) write(`${line}\n`);
  };

  try {
    throwIfAborted(signal);
    for await (const raw of lines) {
      throwIfAborted(signal);
      const line = raw.trim();
      if (line === "") continue;

      if (line.startsWith("/")) {
        if ((await handleCommand(line, session, {
          emit,
          signal,
          compact: () => compactSession(session, { signal }),
        })) === "exit") break;
        continue;
      }

      write(`> ${terminalText(line)}\n`);
      const parentId = session.conversation.activeNodeId;
      const createdAt = new Date().toISOString();
      const history = session.conversation.history;
      const modelHistory = session.conversation.contextHistory;
      const before = history.length;
      const prospectiveNodeId = session.conversation.nodes.length + 1;
      let nodeId: number | undefined;
      let context: ContextAnchor | undefined;
      const policy = (): Promise<ContextPolicy> => {
        return resolveContextPolicy({
          provider: session.provider,
          model: session.model,
          compactionPercent: session.config.compactionPercent,
        });
      };
      const user = { role: "user" as const, content: [{ kind: "text" as const, text: line }] };
      history.push(user);
      modelHistory.push(structuredClone(user));

      const turn = events(emit, session);
      const commit = (checkpoint: readonly Message[], settlement: "checkpointed" | "completed") => {
        session.conversation = session.conversation.commit({
          ...(nodeId === undefined ? {} : { nodeId }),
          parentId,
          createdAt,
          identity: {
            providerId: session.provider.id,
            model: session.model,
            effort: session.config.effort,
          },
          messages: checkpoint.slice(before),
          blocks: [],
          ...(context === undefined ? {} : { context }),
        }, settlement);
        nodeId = session.conversation.activeNodeId;
      };

      const compact = async (
        checkpoint: readonly Message[],
        projected: readonly Message[],
        request: ContextRequest,
      ) => {
        if (
          request.reason === "overflow" &&
          (request.error === undefined || !isContextOverflow(request.error))
        ) {
          return undefined;
        }
        const force = request.reason === "overflow";
        const result = await compactContext({
          provider: session.provider,
          model: session.model,
          effort: session.config.effort,
          context: projected,
          turn: checkpoint.slice(before),
          nodeId: nodeId ?? prospectiveNodeId,
          coveredMessages: context?.messageCount ?? 0,
          lastInputTokens: Math.max(session.usage.lastInputTokens, request.inputTokens),
          force,
          policy: request.policy,
        });
        if (result === undefined) return undefined;
        context = result.anchor;
        if (result.usage !== undefined) recordAuxiliaryUsage(session.usage, result.usage);
        return result.messages;
      };

      turn.onContext = compact;
      turn.onCheckpoint = async (checkpoint, settlement, projected) => {
        commit(checkpoint, settlement);
        const compacted = await compact(checkpoint, projected, {
          reason: "budget",
          policy: await policy(),
          inputTokens: session.usage.lastInputTokens,
        });
        if (compacted !== undefined) commit(checkpoint, settlement);
        return compacted;
      };
      await runTurn(history, options(session, policy), turn, signal, modelHistory);
      turn.flush();
    }
    throwIfAborted(signal);
  } finally {
    signal?.removeEventListener("abort", stopInput);
    rl?.close();
  }
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted === true) throw signal.reason;
}

function options(session: Session, contextPolicy: () => Promise<ContextPolicy>) {
  return {
    provider: session.provider,
    tools: session.tools,
    model: session.model,
    system: session.system,
    maxTokens: session.config.maxTokens,
    contextPolicy,
    effort: session.config.effort,
    maxModelRequests: session.config.maxModelRequests,
    toolContext: { root: session.config.root },
  };
}

// Prose is buffered rather than streamed: without a screen to repaint there is
// nothing to gain from partial lines, and plenty to lose in readability.
function events(
  emit: (block: Block) => void,
  session: Session,
): ControllerEvents & { flush(): void } {
  let answer = "";

  const flush = (): void => {
    if (answer === "") return;
    emit({ kind: "answer", text: answer });
    answer = "";
  };

  return {
    flush,
    onStream(event) {
      if (event.kind === "text") answer += event.text;
    },
    onToolCall() {
      flush();
    },
    onToolResult(call, result, summary) {
      emit({
        kind: "tool",
        name: call.name,
        target: "",
        right: summary ?? "",
        tone: result.isError ? "fail" : "ok",
      });
    },
    approve() {
      flush();
      // Nobody is watching a pipe. Approval has to be granted up front with
      // --auto-approve, never inferred from silence.
      return Promise.resolve(session.config.autoApprove);
    },
    onUsage(usage) {
      recordUsage(session.usage, usage);
    },
    onRequestInput(inputTokens) {
      recordRequestInput(session.usage, inputTokens);
    },
  };
}
