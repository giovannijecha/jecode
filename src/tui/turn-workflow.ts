// One model turn, including steering, compaction, durable settlement, and recovery.

import { runTurn } from "../controller.ts";
import type { ContextRequest } from "../controller.ts";
import type { TurnFailure, TurnSettlement } from "../conversation.ts";
import { resolveContextPolicy } from "../context/capacity.ts";
import { automaticCompactionKey } from "../context/automatic.ts";
import type { AutomaticCompactionGate } from "../context/automatic.ts";
import { compactContext } from "../context/compactor.ts";
import type { ContextAnchor } from "../context/projection.ts";
import type { ContextPolicy } from "../context/policy.ts";
import { isContextOverflow } from "../context/policy.ts";
import { steeringInbox } from "../steering.ts";
import type { SteeringInbox } from "../steering.ts";
import { recordAuxiliaryUsage, recordRequestInput, recordUsage } from "../usage.ts";
import type { Message } from "../types.ts";
import { toolSpecs } from "../tools/index.ts";
import { requestIdentityForSession } from "../request-identity.ts";
import { transition } from "./activity.ts";
import type { AppActions } from "./app-input.ts";
import type { AppState } from "./app-state.ts";
import { answerAt } from "./approve.ts";
import * as edit from "./editor.ts";
import { controllerOptions, turnFailure } from "./session-view.ts";
import { transcribe } from "./turn.ts";
import type { WorkflowOptions } from "./workflow-types.ts";

const WAITING = "Waiting";

export function turnWorkflow(
  options: WorkflowOptions,
  automaticCompaction: AutomaticCompactionGate,
): Pick<AppActions, "turn" | "steer"> {
  const { session, state, permissions, feedback } = options;
  let activeSteering: SteeringInbox | undefined;

  async function turn(text: string): Promise<void> {
    const activity = options.startActivity("turn", WAITING);
    if (activity === undefined) return;
    const inbox = steeringInbox((pending, accepting) => {
      if (activeSteering !== inbox) return;
      state.steering = accepting ? pending : undefined;
      options.render();
    });
    activeSteering = inbox;
    state.steering = 0;
    const status = (label: string): void => transition(activity, label);
    const parentId = session.conversation.activeNodeId;
    const history = session.conversation.history;
    const modelHistory = session.conversation.contextHistory;
    const historyStart = history.length;
    const blockStart = state.blocks.length;
    const createdAt = new Date().toISOString();
    const prospectiveNodeId = session.conversation.nodes.length + 1;
    let nodeId: number | undefined;
    let context: ContextAnchor | undefined;
    const unpersistedSteering: string[] = [];
    const turnTools = permissions.availableTools();
    const specs = toolSpecs(turnTools);
    let firstPolicy = true;
    const policy = (): Promise<ContextPolicy> => {
      let visible = firstPolicy;
      firstPolicy = false;
      if (visible) {
        status("Checking context");
        options.render();
      }
      return resolveContextPolicy({
        provider: session.provider,
        model: session.model,
        compactionPercent: session.config.compactionPercent,
        signal: activity.control.signal,
        onStatus: (said) => {
          visible = true;
          status(said);
          options.render();
        },
      }).finally(() => {
        if (visible) {
          status(WAITING);
          options.render();
        }
      });
    };
    options.emit({ kind: "user", text });
    const user = { role: "user" as const, content: [{ kind: "text" as const, text }] };
    history.push(user);
    modelHistory.push(structuredClone(user));

    const events = transcribe({
      emit: options.emit,
      render: options.render,
      palette: session.palette,
      approved: (call) => permissions.approved(call),
      remember: (call) => permissions.remember(call),
      ask: (prompt, settle) => {
        state.open = { picker: prompt, settle: (index?: number) => settle(answerAt(index)) };
        options.render();
      },
      status: (text) => {
        status(text);
      },
      usage: (usage) => recordUsage(session.usage, usage),
      requestInput: (inputTokens) => recordRequestInput(session.usage, inputTokens),
    });
    const persist = async (
      checkpoint: readonly (typeof history)[number][],
      settlement: TurnSettlement,
      failure?: TurnFailure,
    ): Promise<void> => {
      const next = session.conversation.commit({
        ...(nodeId === undefined ? {} : { nodeId }),
        parentId,
        createdAt,
        identity: {
          providerId: session.provider.id,
          model: session.model,
          effort: session.config.effort,
        },
        messages: checkpoint.slice(historyStart),
        blocks: state.blocks.slice(blockStart),
        ...(context === undefined ? {} : { context }),
        ...(failure === undefined ? {} : { failure }),
      }, settlement);
      await session.persistence?.checkpoint(next);
      session.conversation = next;
      nodeId = next.activeNodeId;
      state.committedNodeId = next.activeNodeId;
      unpersistedSteering.length = 0;
    };

    const compact = async (
      checkpoint: readonly (typeof history)[number][],
      projected: readonly (typeof history)[number][],
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
        turn: checkpoint.slice(historyStart),
        nodeId: nodeId ?? prospectiveNodeId,
        coveredMessages: context?.messageCount ?? 0,
        lastInputTokens: Math.max(session.usage.lastInputTokens, request.inputTokens),
        estimatedInputTokens: request.inputTokens,
        signal: activity.control.signal,
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
          status("Compacting");
          options.render();
        },
        onEnd: () => {
          status(WAITING);
          options.render();
        },
      });
      if (result === undefined) {
        if (attempted && !activity.control.signal.aborted) automaticCompaction.failed(attempt);
        return undefined;
      }
      automaticCompaction.succeeded(attempt);
      context = result.anchor;
      if (result.usage !== undefined) recordAuxiliaryUsage(session.usage, result.usage);
      return result.messages;
    };

    events.onContext = compact;
    events.onSteering = (guidance) => {
      unpersistedSteering.push(guidance);
      options.emit({ kind: "user", text: guidance });
      options.render();
    };
    events.onCheckpoint = async (checkpoint, settlement, projected) => {
      await persist(checkpoint, settlement);
      const compacted = await compact(checkpoint, projected, {
        reason: "budget",
        policy: await policy(),
        inputTokens: session.usage.lastInputTokens,
        projectionSaturated: false,
      });
      if (compacted !== undefined) await persist(checkpoint, settlement);
      return compacted;
    };

    let finishReason: "interrupted" | "failed" | undefined;
    let failed: { error: Error; interrupted: boolean } | undefined;
    try {
      await runTurn(
        history,
        controllerOptions(session, policy, turnTools, inbox),
        events,
        activity.control.signal,
        modelHistory,
      );
    } catch (error) {
      const interrupted = activity.control.signal.aborted;
      const completed = nodeId !== undefined && session.conversation.activeNodeId === nodeId &&
        session.conversation.activeNode?.settlement === "completed";
      if (completed) {
        const notice = turnFailure(session, error as Error, interrupted);
        feedback.show({ text: notice.text, tone: notice.tone, timeoutMs: 6_000 });
      } else {
        finishReason = interrupted ? "interrupted" : "failed";
        failed = { error: error as Error, interrupted };
      }
    } finally {
      let pendingSteering = inbox.close();
      if (activeSteering === inbox) activeSteering = undefined;
      state.steering = undefined;
      try {
        events.finish(finishReason);
        if (failed !== undefined) {
          const notice = turnFailure(session, failed.error, failed.interrupted);
          const settlement = failed.interrupted ? "interrupted" : "failed";
          const failure: TurnFailure = {
            text: notice.text,
            tone: failed.interrupted ? "warn" : "error",
          };
          options.emit(notice);
          try {
            await persist(closeFailedTurn(history, settlement), settlement, failure);
          } catch (error) {
            // A failed persistence boundary cannot remain visible as if it had
            // been saved. Revert to the last durable path and return the input
            // to the composer so the user can retry without losing it.
            options.replaceTranscript();
            state.editor = edit.of([
              ...(nodeId === undefined ? [text] : []),
              ...unpersistedSteering,
              ...pendingSteering,
              state.editor.text,
            ].filter((part) => part !== "").join("\n\n"));
            state.completing = undefined;
            pendingSteering = [];
            feedback.show({
              text: (error as Error).message,
              tone: "error",
              timeoutMs: 6_000,
            });
          }
        }
      } finally {
        restorePendingSteering(state, pendingSteering);
        options.finishActivity(activity);
      }
    }
  }

  function steer(text: string) {
    return activeSteering?.offer(text) ?? "unavailable";
  }

  return { turn, steer };
}

function restorePendingSteering(state: AppState, messages: readonly string[]): void {
  if (messages.length === 0) return;
  const pending = messages.join("\n\n");
  state.editor = edit.of(state.editor.text === "" ? pending : `${pending}\n\n${state.editor.text}`);
  state.completing = undefined;
}

function closeFailedTurn(history: readonly Message[], settlement: "failed" | "interrupted"): Message[] {
  const closed = [...history];
  if (closed.at(-1)?.role === "assistant") return closed;
  const text = settlement === "interrupted"
    ? "The previous attempt was interrupted by the user before completion."
    : "The previous attempt failed before completion.";
  closed.push({ role: "assistant", content: [{ kind: "text", text }] });
  return closed;
}
