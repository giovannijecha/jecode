// Foreground command and model-turn workflows for the TUI shell.

import { handleCommand } from "../commands.ts";
import { runTurn } from "../controller.ts";
import type { Session } from "../session.ts";
import { updateSettings } from "../settings.ts";
import { saveTranscript } from "../transcript-export.ts";
import { recordUsage } from "../usage.ts";
import type { Activity } from "./activity.ts";
import type { AppActions } from "./app-input.ts";
import type { AppState } from "./app-state.ts";
import { answerAt, scopeFor } from "./approve.ts";
import type { Block, NoticeBlock } from "./blocks.ts";
import type { FeedbackController } from "./feedback.ts";
import { controllerOptions, turnFailure } from "./session-view.ts";
import { transcribe } from "./turn.ts";

const WAITING = "Waiting";

type WorkflowOptions = {
  session: Session;
  transcriptRoot: string;
  state: AppState;
  allowed: Map<string, string>;
  feedback: FeedbackController;
  emit(block: Block): void;
  commandNotice(notice: NoticeBlock): void;
  render(block?: Block): void;
  refreshSettings(): void;
  startActivity(kind: Activity["kind"], label: string): Activity | undefined;
  finishActivity(activity: Activity): void;
};

export function appWorkflows(options: WorkflowOptions): AppActions {
  const { session, state, allowed, feedback } = options;

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
        type: (field) =>
          new Promise<string | undefined>((resolve) => {
            state.open = { field, settle: resolve };
            options.render();
          }),
        status: (said) => {
          state.status = said ?? activity.label;
          options.render();
        },
        reset: () => {
          state.blocks.splice(0);
          state.past.length = 0;
          allowed.clear();
          state.scroll = 0;
          state.follow = true;
          state.unseen = 0;
          state.lastMaxScroll = 0;
        },
        permissions: () => [...allowed].map(([key, label]) => ({ key, label })),
        revokePermission: (key) => {
          if (key === undefined) allowed.clear();
          else allowed.delete(key);
        },
        exportTranscript: () => saveTranscript(options.transcriptRoot, state.blocks),
        saveSettings: async (patch) => {
          await updateSettings(patch);
        },
        refreshSettings: options.refreshSettings,
      });
      if (outcome === "exit") state.closeWhenIdle = true;
    } catch (error) {
      feedback.show({
        text: activity.control.signal.aborted ? "interrupted" : (error as Error).message,
        tone: activity.control.signal.aborted ? "warn" : "error",
        timeoutMs: activity.control.signal.aborted ? 4_200 : 6_000,
      });
    } finally {
      options.finishActivity(activity);
    }
  }

  async function turn(text: string): Promise<void> {
    const activity = options.startActivity("turn", WAITING);
    if (activity === undefined) return;
    options.emit({ kind: "user", text });
    session.history.push({ role: "user", content: [{ kind: "text", text }] });

    const events = transcribe({
      emit: options.emit,
      render: options.render,
      palette: session.palette,
      approved: (call) => session.config.autoApprove || allowed.has(scopeFor(call).key),
      remember: (call) => {
        const scope = scopeFor(call);
        allowed.set(scope.key, scope.summary);
      },
      ask: (prompt, settle) => {
        state.open = { picker: prompt, settle: (index?: number) => settle(answerAt(index)) };
        options.render();
      },
      status: (text) => {
        state.status = text;
      },
      usage: (usage) => recordUsage(session.usage, usage),
    });

    let finishReason: "interrupted" | "failed" | undefined;
    try {
      await runTurn(session.history, controllerOptions(session), events, activity.control.signal);
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
