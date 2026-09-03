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
import type { ContextPolicy } from "./context/policy.ts";
import type { SteeringSource } from "./steering.ts";
import type { Tool, ToolContext, ToolPreview, ToolRun } from "./tools/index.ts";
import { findTool, runTool, toolSpecs } from "./tools/index.ts";
import { requestAssistant } from "./controller-request.ts";

export const MAX_TOOL_CALLS_PER_RESPONSE = 32;
/** Independent read calls share one bounded execution wave. */
export const MAX_CONCURRENT_TOOL_CALLS = 4;

export type ContextReason = "budget" | "overflow";

export type ContextRequest = Readonly<{
  reason: ContextReason;
  policy: ContextPolicy;
  inputTokens: number;
  error?: Error;
}>;

export type ControllerOptions = {
  provider: Provider;
  tools: Tool[];
  model: string;
  system: string;
  maxTokens: number;
  /** Resolve for each provider attempt; adapters cache stable metadata themselves. */
  contextPolicy(): Promise<ContextPolicy>;
  effort: string;
  /** Stop after this many model requests when an explicit launch budget is set. */
  maxModelRequests?: number;
  toolContext: ToolContext;
  /** Cooperative user guidance consumed only at safe request boundaries. */
  steering?: SteeringSource;
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
  /** Best context-pressure signal: provider count when present, otherwise the sent estimate. */
  onRequestInput?(inputTokens: number): void;
  /** A queued user message has entered canonical history for this turn. */
  onSteering?(text: string): void;
  /** A tool call is complete on the wire and its local preview is being prepared. */
  onToolPreparing?(call: ToolCallBlock, current: number, total: number): void;
  /** Approval and validation are complete; execution is about to begin. */
  onToolStart?(call: ToolCallBlock, current: number, total: number): void;
  onStatus?(status: string): void;
  /** Replace only the provider-facing projection, never canonical history. */
  onContext?(
    history: readonly Message[],
    context: readonly Message[],
    request: ContextRequest,
  ): Promise<readonly Message[] | undefined>;
  /** A consistent history boundary, awaited before another request can open. */
  onCheckpoint?(
    history: readonly Message[],
    settlement: "checkpointed" | "completed",
    context: readonly Message[],
  ): Promise<readonly Message[] | void>;
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
  modelHistory: Message[] = history,
): Promise<void> {
  const specs = toolSpecs(options.tools);
  let context = modelHistory;
  throwIfAborted(signal);

  const append = (message: Message): void => {
    history.push(message);
    if (context !== history) context.push(message);
  };

  const checkpoint = async (settlement: "checkpointed" | "completed"): Promise<void> => {
    const projected = await events.onCheckpoint?.(history, settlement, context);
    if (projected !== undefined) context = clone([...projected]);
  };

  const appendSteering = (messages: readonly string[]): boolean => {
    for (const text of messages) {
      append({ role: "user", content: [{ kind: "text", text }] });
    }
    for (const text of messages) {
      events.onSteering?.(text);
    }
    return messages.length > 0;
  };

  let requests = 0;
  while (true) {
    throwIfAborted(signal);
    if (
      options.maxModelRequests !== undefined &&
      requests >= options.maxModelRequests
    ) {
      const requestLabel = options.maxModelRequests === 1 ? "request" : "requests";
      throw new Error(
        `stopped after ${options.maxModelRequests} model ${requestLabel} (--max-steps limit reached)`,
      );
    }
    appendSteering(options.steering?.drain() ?? []);
    requests++;
    // The message is displayed as it streams; what comes back here is the
    // assembled version, which exists to be appended to the history.
    const response = await requestAssistant(
      history,
      context,
      specs,
      options,
      events,
      signal,
    );
    const assistant = response.message;
    context = response.context;
    throwIfAborted(signal);

    const calls = assistant.content.filter(isToolCall);
    if (assistant.content.length === 0) {
      throw new Error(`${options.provider.id} completed without an answer or tool call`);
    }
    if (calls.length > MAX_TOOL_CALLS_PER_RESPONSE) {
      throw new Error(
        `provider returned ${calls.length} tool calls in one response (maximum ${MAX_TOOL_CALLS_PER_RESPONSE})`,
      );
    }
    assertToolCallIds(calls);
    if (assistant.usage !== undefined) events.onUsage?.(assistant.usage);
    events.onRequestInput?.(
      assistant.usage !== undefined && assistant.usage.inputTokens > 0
        ? assistant.usage.inputTokens
        : response.inputTokens,
    );

    append(assistant);
    if (calls.length === 0) {
      const steering = options.steering?.drainOrClose();
      if (steering !== undefined && appendSteering(steering.messages)) {
        await checkpoint("checkpointed");
        continue;
      }
      await checkpoint("completed");
      return; // the model is done — hand back to the user
    }

    // Consecutive shared reads run together. An exclusive call is an ordered
    // barrier, so writes, approvals, and commands never overlap other work.
    // Every result still goes back in a SINGLE message and in call order.
    const results: ToolResultBlock[] = [];
    const announced = new Set<string>();
    try {
      while (results.length < calls.length) {
        const start = results.length;
        const batch = nextBatch(calls, start, options.tools);
        const prepared: {
          call: ToolCallBlock;
          current: number;
          preview?: ToolPreview;
        }[] = [];

        for (let offset = 0; offset < batch.length; offset++) {
          throwIfAborted(signal);
          const call = batch[offset] as ToolCallBlock;
          const current = start + offset + 1;
          events.onToolPreparing?.(call, current, calls.length);
          const preview = await look(call, options, signal);
          throwIfAborted(signal);
          announced.add(call.id);
          events.onToolCall(call, preview);
          prepared.push({ call, current, preview });
        }

        const runs = await Promise.all(
          prepared.map(({ call, current, preview }) =>
            settle(call, current, calls.length, options, events, signal, preview)
          ),
        );
        for (let offset = 0; offset < runs.length; offset++) {
          const call = prepared[offset]?.call as ToolCallBlock;
          const run = runs[offset] as ToolRun;
          results.push(run.result);
          events.onToolResult(call, run.result, run.summary);
        }
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
      append({ role: "user", content: results });
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
      await checkpoint("checkpointed");
      if (interrupted) throw abortReason(signal as AbortSignal);
      throw error;
    }

    append({ role: "user", content: results });
    appendSteering(options.steering?.drain() ?? []);
    await checkpoint("checkpointed");
  }
}

function nextBatch(
  calls: readonly ToolCallBlock[],
  start: number,
  tools: Tool[],
): ToolCallBlock[] {
  const first = calls[start];
  if (first === undefined) return [];
  if (!shared(findTool(tools, first.name))) return [first];

  const batch: ToolCallBlock[] = [];
  for (
    let index = start;
    index < calls.length && batch.length < MAX_CONCURRENT_TOOL_CALLS;
    index++
  ) {
    const call = calls[index] as ToolCallBlock;
    if (!shared(findTool(tools, call.name))) break;
    batch.push(call);
  }
  return batch;
}

function shared(tool: Tool | undefined): boolean {
  return tool?.concurrency === "shared" && !tool.dangerous;
}

function assertToolCallIds(calls: readonly ToolCallBlock[]): void {
  const seen = new Set<string>();
  for (const call of calls) {
    if (call.id.trim() === "") throw new Error("provider returned a tool call without an id");
    if (seen.has(call.id)) {
      throw new Error("provider returned duplicate tool call ids in one response");
    }
    seen.add(call.id);
  }
}

async function settle(
  call: ToolCallBlock,
  current: number,
  total: number,
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
  events.onToolStart?.(call, current, total);
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

function clone<T>(value: T): T {
  return structuredClone(value);
}
