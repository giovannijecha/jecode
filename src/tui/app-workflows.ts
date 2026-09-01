// Foreground command and model-turn workflows for the TUI shell.

import { handleCommand } from "../commands.ts";
import { runTurn } from "../controller.ts";
import { resolveContextPolicy } from "../context/capacity.ts";
import { compactContext } from "../context/compactor.ts";
import type { ContextAnchor } from "../context/projection.ts";
import type { ContextPolicy } from "../context/policy.ts";
import { isContextOverflow, shouldResolveContextPolicy } from "../context/policy.ts";
import type { Session } from "../session.ts";
import { updateSettings } from "../settings.ts";
import { saveTranscript } from "../transcript-export.ts";
import { recordAuxiliaryUsage, recordUsage } from "../usage.ts";
import type { SessionPermissions } from "../permissions.ts";
import type { Activity } from "./activity.ts";
import type { AppActions } from "./app-input.ts";
import type { AppState } from "./app-state.ts";
import { answerAt } from "./approve.ts";
import type { Block, NoticeBlock } from "./blocks.ts";
import type { FeedbackController } from "./feedback.ts";
import { cancel as cancelOpen } from "./overlay.ts";
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
  refreshSettings(): void;
  startActivity(kind: Activity["kind"], label: string): Activity | undefined;
  finishActivity(activity: Activity): void;
};

export function appWorkflows(options: WorkflowOptions): AppActions {
  const { session, state, permissions, feedback } = options;

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
        choose: (picker) =>
          new Promise<number | undefined>((resolve) => {
            state.open = { picker, settle: resolve };
            options.render();
          }),
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
        },
        permissions,
        exportTranscript: () => saveTranscript(options.transcriptRoot, state.blocks),
        saveSettings: async (patch) => {
          await updateSettings(patch);
        },
        refreshSettings: options.refreshSettings,
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
    let contextPolicy: Promise<ContextPolicy> | undefined;
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
    });
    const persist = async (
      checkpoint: readonly (typeof history)[number][],
      settlement: "checkpointed" | "completed",
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
      }, settlement);
      await session.persistence?.checkpoint(next);
      session.conversation = next;
      nodeId = next.activeNodeId;
    };

    const compact = async (
      checkpoint: readonly (typeof history)[number][],
      projected: readonly (typeof history)[number][],
      reason: "budget" | "overflow",
      error?: Error,
    ) => {
      if (reason === "overflow" && (error === undefined || !isContextOverflow(error))) {
        return undefined;
      }
      const force = reason === "overflow";
      if (!shouldResolveContextPolicy(projected, session.usage.lastInputTokens, force)) {
        return undefined;
      }
      if (contextPolicy === undefined) {
        state.status = "Checking context";
        options.render();
        contextPolicy = resolveContextPolicy({
          provider: session.provider,
          model: session.model,
          compactionPercent: session.config.compactionPercent,
          signal: activity.control.signal,
          onStatus: (status) => {
            state.status = status;
            options.render();
          },
        }).finally(() => {
          state.status = WAITING;
          options.render();
        });
      }
      const result = await compactContext({
        provider: session.provider,
        model: session.model,
        effort: session.config.effort,
        context: projected,
        turn: checkpoint.slice(historyStart),
        nodeId: nodeId ?? prospectiveNodeId,
        coveredMessages: context?.messageCount ?? 0,
        lastInputTokens: session.usage.lastInputTokens,
        signal: activity.control.signal,
        force,
        policy: await contextPolicy,
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
      const compacted = await compact(checkpoint, projected, "budget");
      if (compacted !== undefined) await persist(checkpoint, settlement);
      return compacted;
    };

    let finishReason: "interrupted" | "failed" | undefined;
    try {
      await runTurn(
        history,
        controllerOptions(session, permissions.availableTools()),
        events,
        activity.control.signal,
        modelHistory,
      );
    } catch (error) {
      const interrupted = activity.control.signal.aborted;
      finishReason = interrupted ? "interrupted" : "failed";
      options.emit(turnFailure(session, error as Error, interrupted));
    } finally {
      events.finish(finishReason);
      options.finishActivity(activity);
    }
  }

  return { command, turn };
}
