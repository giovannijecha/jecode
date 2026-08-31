// The single controller.
//
// One loop, in dialogue with the user. It never delegates: no subagent, no
// worker, no orchestrator split, no second loop. When a task is too big, this
// loop iterates. That constraint is implemented here, not left as a product claim.

import type {
  Message,
  Provider,
  StreamEvent,
  ToolCallBlock,
  ToolResultBlock,
  Usage,
} from "./types.ts";
import { isToolCall } from "./types.ts";
import type { Tool, ToolContext, ToolPreview, ToolRun } from "./tools/index.ts";
import { findTool, runTool, toolSpecs } from "./tools/index.ts";

export const MAX_TOOL_CALLS_PER_STEP = 32;

export type ControllerOptions = {
  provider: Provider;
  tools: Tool[];
  model: string;
  system: string;
  maxTokens: number;
  effort: string;
  maxSteps: number;
  toolContext: ToolContext;
};

export type ControllerEvents = {
  /** Text and reasoning as they arrive. This is the only path text is shown. */
  onStream(event: StreamEvent): void;
  /** `look` is what the call would change, when the tool can say so. */
  onToolCall(call: ToolCallBlock, look?: ToolPreview): void;
  /** `summary` is the tool's own one-phrase measure of what it did. */
  onToolResult(call: ToolCallBlock, result: ToolResultBlock, summary?: string): void;
  /** A safe, bounded snapshot of output from a tool that can report progress. */
  onToolOutput?(call: ToolCallBlock, output: string): void;
  approve(call: ToolCallBlock): Promise<boolean>;
  onUsage?(usage: Usage): void;
  onStep?(step: number, total: number): void;
  onToolProgress?(current: number, total: number): void;
  onStatus?(status: string): void;
};

/**
 * Run one user turn to completion: keep exchanging with the model until it
 * stops asking for tools. `history` is mutated in place, so an aborted turn
 * still leaves the conversation in a consistent state.
 */
export async function runTurn(
  history: Message[],
  options: ControllerOptions,
  events: ControllerEvents,
  signal?: AbortSignal,
): Promise<void> {
  const specs = toolSpecs(options.tools);

  for (let step = 0; step < options.maxSteps; step++) {
    throwIfAborted(signal);
    events.onStep?.(step + 1, options.maxSteps);
    // The message is displayed as it streams; what comes back here is the
    // assembled version, which exists to be appended to the history.
    const assistant = await options.provider.send({
      model: options.model,
      system: options.system,
      messages: history,
      tools: specs,
      maxTokens: options.maxTokens,
      effort: options.effort,
      signal,
      onStream: (event) => events.onStream(event),
      onStatus: (status) => events.onStatus?.(status),
    });
    throwIfAborted(signal);

    const calls = assistant.content.filter(isToolCall);
    if (assistant.content.length === 0) {
      throw new Error(`${options.provider.id} completed without an answer or tool call`);
    }
    if (calls.length > MAX_TOOL_CALLS_PER_STEP) {
      throw new Error(
        `provider returned ${calls.length} tool calls in one step (maximum ${MAX_TOOL_CALLS_PER_STEP})`,
      );
    }
    assertToolCallIds(calls);

    history.push(assistant);
    if (calls.length === 0) {
      if (assistant.usage !== undefined) events.onUsage?.(assistant.usage);
      return; // the model is done — hand back to the user
    }

    // Calls run one after another because approval prompts serialise anyway,
    // but every result from this step goes back in a SINGLE message. Splitting
    // them teaches the model to stop batching its calls.
    const results: ToolResultBlock[] = [];
    const announced = new Set<string>();
    try {
      if (assistant.usage !== undefined) events.onUsage?.(assistant.usage);
      for (let index = 0; index < calls.length; index++) {
        throwIfAborted(signal);
        const call = calls[index] as ToolCallBlock;
        events.onToolProgress?.(index + 1, calls.length);
        const preview = await look(call, options, signal);
        throwIfAborted(signal);
        announced.add(call.id);
        events.onToolCall(call, preview);
        const { result, summary } = await settle(call, options, events, signal, preview);
        results.push(result);
        events.onToolResult(call, result, summary);
      }
    } catch (error) {
      const interrupted = signal?.aborted === true;
      const repairs: { call: ToolCallBlock; run: ToolRun }[] = [];
      for (const call of calls.slice(results.length)) {
        const run = interrupted
          ? refuse(call, "interrupted before completion", "interrupted")
          : refuse(call, "tool processing stopped before completion", "failed");
        repairs.push({ call, run });
        results.push(run.result);
      }
      history.push({ role: "user", content: results });
      // History repair is the invariant. UI recovery is best-effort and must
      // never replace the original exception or leave the conversation open.
      for (const { call, run } of repairs) {
        if (!announced.has(call.id)) continue;
        try {
          events.onToolResult(call, run.result, run.summary);
        } catch {
          // The surface is already failing; the next turn can still proceed.
        }
      }
      if (interrupted) throw abortReason(signal as AbortSignal);
      throw error;
    }

    history.push({ role: "user", content: results });
  }

  throw new Error(
    `gave up after ${options.maxSteps} steps without finishing (raise --max-steps)`,
  );
}

function assertToolCallIds(calls: readonly ToolCallBlock[]): void {
  const seen = new Set<string>();
  for (const call of calls) {
    if (call.id.trim() === "") throw new Error("provider returned a tool call without an id");
    if (seen.has(call.id)) {
      throw new Error("provider returned duplicate tool call ids in one step");
    }
    seen.add(call.id);
  }
}

async function settle(
  call: ToolCallBlock,
  options: ControllerOptions,
  events: ControllerEvents,
  signal: AbortSignal | undefined,
  preview: ToolPreview | undefined,
): Promise<ToolRun> {
  const tool = findTool(options.tools, call.name);

  if (tool === undefined) {
    return refuse(call, `no such tool: ${call.name}`, "unknown tool");
  }
  throwIfAborted(signal);
  const approved = !tool.dangerous || await events.approve(call);
  throwIfAborted(signal);
  if (!approved) {
    return refuse(call, "the user declined this call — ask them how to proceed", "declined");
  }

  throwIfAborted(signal);
  return runTool(tool, call, {
    ...options.toolContext,
    signal,
    preview,
    onOutput: (output) => events.onToolOutput?.(call, output),
  });
}

function refuse(call: ToolCallBlock, reason: string, summary: string): ToolRun {
  return {
    result: { kind: "tool_result", id: call.id, output: reason, isError: true },
    summary,
  };
}

/**
 * What the call would change, asked of the tool that would change it.
 *
 * Read-only and best-effort by construction: a preview that throws — a missing
 * file, a match that is not there — is simply no preview. Nothing about the
 * turn depends on it, because it exists for the user, not for the model.
 */
async function look(
  call: ToolCallBlock,
  options: ControllerOptions,
  signal: AbortSignal | undefined,
): Promise<ToolPreview | undefined> {
  const tool = findTool(options.tools, call.name);
  if (tool?.preview === undefined) return undefined;

  try {
    return await tool.preview(call.input, { ...options.toolContext, signal });
  } catch {
    return undefined;
  }
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted === true) throw abortReason(signal);
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error("interrupted");
}
