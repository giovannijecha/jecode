// The non-interactive path: stdin is a pipe, so there is no screen to own.
//
// It exists so the agent can be scripted and tested. It shares the controller,
// the tools and the block renderer with the TUI — only the surface differs.

import { stdin, stdout } from "node:process";
import type { Message } from "./types.ts";
import type { Session } from "./session.ts";
import type { ContextRequest, ControllerEvents } from "./controller.ts";
import { runTurn } from "./controller.ts";
import { resolveContextPolicy } from "./context/capacity.ts";
import {
  automaticCompactionGate,
  automaticCompactionKey,
} from "./context/automatic.ts";
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
import { assertPromptLength, boundedInputLines } from "./input-boundary.ts";
import { providerFailure } from "./provider-errors.ts";
import { toolSpecs } from "./tools/index.ts";
import { requestIdentityForSession } from "./request-identity.ts";

export type BatchEnvironment = {
  lines?: AsyncIterable<string>;
  write?(text: string): void;
  width?: number;
  signal?: AbortSignal;
};

export async function runBatch(session: Session, environment: BatchEnvironment = {}): Promise<void> {
  const write = environment.write ?? ((text: string) => stdout.write(text));
  const width = environment.width ?? columns();
  const signal = environment.signal;
  const source = environment.lines ?? boundedInputLines(stdin);
  const lines = abortableLines(source, signal);
  const automaticCompaction = automaticCompactionGate();

  const emit = (block: Block): void => {
    for (const line of renderBatch(block, width, session.palette)) write(`${line}\n`);
  };

  try {
    throwIfAborted(signal);
    for await (const raw of lines) {
      throwIfAborted(signal);
      assertPromptLength(raw.length);
      const line = raw.trim();
      if (line === "") continue;

      if (line.startsWith("/")) {
        if ((await handleCommand(line, session, {
          emit,
          signal,
          reset: () => {
            automaticCompaction.reset();
          },
          compact: async () => {
            const result = await compactSession(session, { signal });
            if (result === "compacted") automaticCompaction.reset();
            return result;
          },
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
      const specs = toolSpecs(session.tools);
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
        const force = request.reason === "overflow" || request.projectionSaturated;
        const key = automaticCompactionKey(
          session.provider.id,
          session.model,
          nodeId ?? prospectiveNodeId,
          checkpoint.length,
        );
        const attempt = { key, reason: request.reason } as const;
        if (!automaticCompaction.allows(attempt)) return undefined;
        let attempted = false;
        const result = await compactContext({
          provider: session.provider,
          model: session.model,
          effort: session.config.effort,
          context: projected,
          turn: checkpoint.slice(before),
          nodeId: nodeId ?? prospectiveNodeId,
          coveredMessages: context?.messageCount ?? 0,
          lastInputTokens: Math.max(session.usage.lastInputTokens, request.inputTokens),
          estimatedInputTokens: request.inputTokens,
          force,
          policy: request.policy,
          requestEnvelope: {
            system: session.system,
            tools: specs,
            maxOutputTokens: session.config.maxTokens,
          },
          requestIdentity: requestIdentityForSession(session),
          onBegin: () => {
            attempted = true;
          },
        });
        if (result === undefined) {
          if (attempted && signal?.aborted !== true) automaticCompaction.failed(attempt);
          return undefined;
        }
        automaticCompaction.succeeded(attempt);
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
          projectionSaturated: false,
        });
        if (compacted !== undefined) commit(checkpoint, settlement);
        return compacted;
      };
      try {
        await runTurn(history, options(session, policy), turn, signal, modelHistory);
      } catch (error) {
        throwIfAborted(signal);
        if (!(error instanceof Error)) throw error;
        const message = providerFailure(session.provider, error, true);
        if (message === error.message) throw error;
        throw new Error(message, { cause: error });
      }
      turn.flush();
    }
    throwIfAborted(signal);
  } finally {
    if (environment.lines === undefined) stdin.pause();
  }
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted === true) throw signal.reason;
}

async function* abortableLines(
  source: AsyncIterable<string>,
  signal: AbortSignal | undefined,
): AsyncGenerator<string> {
  const iterator = source[Symbol.asyncIterator]();
  let exhausted = false;
  try {
    while (true) {
      throwIfAborted(signal);
      const next = signal === undefined
        ? await iterator.next()
        : await new Promise<IteratorResult<string>>((resolve, reject) => {
          const abort = (): void => reject(signal.reason);
          signal.addEventListener("abort", abort, { once: true });
          iterator.next().then(
            (result) => {
              signal.removeEventListener("abort", abort);
              resolve(result);
            },
            (error: unknown) => {
              signal.removeEventListener("abort", abort);
              reject(error);
            },
          );
        });
      if (next.done === true) {
        exhausted = true;
        return;
      }
      yield next.value;
    }
  } finally {
    if (!exhausted && iterator.return !== undefined) {
      const closing = iterator.return();
      if (signal?.aborted === true) void closing.catch(() => {});
      else await closing;
    }
  }
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
    requestIdentity: requestIdentityForSession(session),
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
