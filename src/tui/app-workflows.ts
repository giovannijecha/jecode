// Foreground command and model-turn workflows for the TUI shell.

import { handleCommand } from "../commands.ts";
import { runTurn } from "../controller.ts";
import type { ContextRequest } from "../controller.ts";
import type { TurnFailure, TurnSettlement } from "../conversation.ts";
import { resolveContextPolicy } from "../context/capacity.ts";
import { compactContext } from "../context/compactor.ts";
import { compactSession } from "../context/manual.ts";
import type { ContextAnchor } from "../context/projection.ts";
import type { ContextPolicy } from "../context/policy.ts";
import { isContextOverflow } from "../context/policy.ts";
import type { Session } from "../session.ts";
import { updateSettings } from "../settings.ts";
import { saveTranscript } from "../transcript-export.ts";
import { recordAuxiliaryUsage, recordRequestInput, recordUsage } from "../usage.ts";
import { selectTimeline } from "../timeline.ts";
import type { SessionPermissions } from "../permissions.ts";
import type { Message } from "../types.ts";
import type { Activity } from "./activity.ts";
import type { AppActions } from "./app-input.ts";
import type { AppState } from "./app-state.ts";
import { answerAt } from "./approve.ts";
import type { Block, NoticeBlock } from "./blocks.ts";
import type { FeedbackController } from "./feedback.ts";
import * as edit from "./editor.ts";
import { cancel as cancelOpen } from "./overlay.ts";
import type { Picker } from "./picker.ts";
import { controllerOptions, turnFailure } from "./session-view.ts";
import { transcribe } from "./turn.ts";

const WAITING = "Waiting";

type WorkflowOptions = {
  session: Session;
  transcriptRoot: string;
  state: AppState;
  permissions: SessionPermissions;
  feedback: FeedbackController;
  emit(block: Block): void;
  commandNotice(notice: NoticeBlock): void;
  render(block?: Block): void;
  replaceTranscript(): void;
  refreshSettings(): void;
  startActivity(kind: Activity["kind"], label: string): Activity | undefined;
  finishActivity(activity: Activity): void;
};

export function appWorkflows(options: WorkflowOptions): AppActions {
  const { session, state, permissions, feedback } = options;

  const choose = (picker: Picker) =>
    new Promise<number | undefined>((resolve) => {
      state.open = { picker, settle: resolve };
      options.render();
    });

  async function command(text: string): Promise<void> {
    const activity = options.startActivity("command", `Running ${text.split(/\s+/)[0]}`);
    if (activity === undefined) return;

    try {
      const outcome = await handleCommand(text, session, {
        emit: options.commandNotice,
        signal: activity.control.signal,
        showHelp: () =>
          new Promise<void>((resolve) => {
            state.open = { help: true, settle: resolve };
            options.render();
          }),
        choose,
        dismiss: () => {
          state.open = state.open === undefined ? undefined : cancelOpen(state.open);
          options.render();
        },
        type: (field) =>
          new Promise<string | undefined>((resolve) => {
            state.open = { field, settle: resolve };
            options.render();
          }),
        status: (said) => {
          state.status = said ?? activity.label;
          options.render();
        },
        reset: async () => {
          await session.persistence?.reset();
          state.blocks.splice(0);
          state.past.length = 0;
          permissions.reset();
          state.scroll = 0;
          state.follow = true;
          state.unseen = 0;
          state.lastMaxScroll = 0;
          state.committedNodeId = 0;
        },
        permissions,
        exportTranscript: () => saveTranscript(options.transcriptRoot, state.blocks),
        saveSettings: async (patch) => {
          await updateSettings(patch);
        },
        refreshSettings: options.refreshSettings,
        timeline: async () => {
          const selected = await selectTimeline(session, choose);
          if (!selected) return "unchanged";
          options.replaceTranscript();
          return session.conversation.activeNodeId === state.committedNodeId
            ? "unchanged"
            : "selected";
        },
        compact: async () => {
          if (session.conversation.activeNodeId !== state.committedNodeId) {
            return "branch-pending";
          }
          return compactSession(session, {
            signal: activity.control.signal,
            onStatus: (status) => {
              state.status = status ?? activity.label;
              options.render();
            },
          });
        },
      });
      if (outcome === "exit") state.closeWhenIdle = true;
    } catch (error) {
      // Command cancellation is already visible through the dock closing and
      // the activity ending. Keep it silent instead of replacing the footer
      // with a redundant warning.
      if (!activity.control.signal.aborted) {
        feedback.show({
          text: (error as Error).message,
          tone: "error",
          timeoutMs: 6_000,
        });
      }
    } finally {
      options.finishActivity(activity);
    }
  }

  async function turn(text: string): Promise<void> {
    const activity = options.startActivity("turn", WAITING);
    if (activity === undefined) return;
    const parentId = session.conversation.activeNodeId;
    const history = session.conversation.history;
    const modelHistory = session.conversation.contextHistory;
    const historyStart = history.length;
    const blockStart = state.blocks.length;
    const createdAt = new Date().toISOString();
    const prospectiveNodeId = session.conversation.nodes.length + 1;
    let nodeId: number | undefined;
    let context: ContextAnchor | undefined;
    let firstPolicy = true;
    const policy = (): Promise<ContextPolicy> => {
      let visible = firstPolicy;
      firstPolicy = false;
      if (visible) {
        state.status = "Checking context";
        options.render();
      }
      return resolveContextPolicy({
        provider: session.provider,
        model: session.model,
        compactionPercent: session.config.compactionPercent,
        signal: activity.control.signal,
        onStatus: (status) => {
          visible = true;
          state.status = status;
          options.render();
        },
      }).finally(() => {
        if (visible) {
          state.status = WAITING;
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
        state.status = text;
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
      const force = request.reason === "overflow";
      const result = await compactContext({
        provider: session.provider,
        model: session.model,
        effort: session.config.effort,
        context: projected,
        turn: checkpoint.slice(historyStart),
        nodeId: nodeId ?? prospectiveNodeId,
        coveredMessages: context?.messageCount ?? 0,
        lastInputTokens: Math.max(session.usage.lastInputTokens, request.inputTokens),
        signal: activity.control.signal,
        force,
        policy: request.policy,
        onBegin: () => {
          state.status = "Compacting";
          options.render();
        },
        onEnd: () => {
          state.status = WAITING;
          options.render();
        },
      });
      if (result === undefined) return undefined;
      context = result.anchor;
      if (result.usage !== undefined) recordAuxiliaryUsage(session.usage, result.usage);
      return result.messages;
    };

    events.onContext = compact;
    events.onCheckpoint = async (checkpoint, settlement, projected) => {
      await persist(checkpoint, settlement);
      const compacted = await compact(checkpoint, projected, {
        reason: "budget",
        policy: await policy(),
        inputTokens: session.usage.lastInputTokens,
      });
      if (compacted !== undefined) await persist(checkpoint, settlement);
      return compacted;
    };

    let finishReason: "interrupted" | "failed" | undefined;
    let failed: { error: Error; interrupted: boolean } | undefined;
    try {
      await runTurn(
        history,
        controllerOptions(session, policy, permissions.availableTools()),
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
            state.editor = edit.of(text);
            feedback.show({
              text: (error as Error).message,
              tone: "error",
              timeoutMs: 6_000,
            });
          }
        }
      } finally {
        options.finishActivity(activity);
      }
    }
  }

  return { command, turn };
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
