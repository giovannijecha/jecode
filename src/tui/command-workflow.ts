// Foreground slash-command interactions and their session updates.

import { commandNeedsIdle, handleCommand } from "../commands.ts";
import { compactSession } from "../context/manual.ts";
import { resetRequestIdentity } from "../request-identity.ts";
import { updateSettings } from "../settings.ts";
import { selectTimeline } from "../timeline.ts";
import { saveTranscript } from "../transcript-export.ts";
import { transition } from "./activity.ts";
import type { AppActions } from "./app-input.ts";
import { cancel as cancelOpen } from "./overlay.ts";
import type { Picker } from "./picker.ts";
import type { WorkflowOptions } from "./workflow-types.ts";

export function commandWorkflow(
  options: WorkflowOptions,
  resetCompaction: () => void,
  resetInput: () => void,
): AppActions["command"] {
  const { session, state, permissions, feedback } = options;

  const choose = (picker: Picker) =>
    new Promise<number | undefined>((resolve) => {
      state.command?.control.signal.throwIfAborted();
      state.open = { picker, settle: resolve };
      options.render();
    });

  async function command(text: string): Promise<void> {
    if (state.activity !== undefined && commandNeedsIdle(text)) return;
    const activity = options.startActivity("command", `Running ${text.split(/\s+/)[0]}`);
    if (activity === undefined) return;
    const status = (label: string): void => transition(activity, label);

    try {
      const outcome = await handleCommand(text, session, {
        emit: options.commandNotice,
        signal: activity.control.signal,
        showHelp: () =>
          new Promise<void>((resolve) => {
            activity.control.signal.throwIfAborted();
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
            activity.control.signal.throwIfAborted();
            state.open = { field, settle: resolve };
            options.render();
          }),
        status: (said) => {
          status(said ?? activity.label);
          options.render();
        },
        reset: async () => {
          await session.persistence?.reset();
          resetRequestIdentity(session);
          resetCompaction();
          state.past.length = 0;
          permissions.reset();
          options.replaceTranscript([]);
          state.committedNodeId = 0;
        },
        permissions,
        exportTranscript: () => saveTranscript(options.transcriptRoot, structuredClone(state.blocks)),
        saveSettings: async (patch) => {
          await updateSettings(patch);
        },
        refreshSettings: options.refreshSettings,
        timeline: async () => {
          const selected = await selectTimeline(session, choose);
          if (!selected) return "unchanged";
          resetCompaction();
          options.replaceTranscript();
          return session.conversation.activeNodeId === state.committedNodeId
            ? "unchanged"
            : "selected";
        },
        compact: async () => {
          if (session.conversation.activeNodeId !== state.committedNodeId) {
            return "branch-pending";
          }
          const result = await compactSession(session, {
            signal: activity.control.signal,
            onStatus: (said) => {
              status(said ?? activity.label);
              options.render();
            },
          });
          if (result === "compacted") resetCompaction();
          return result;
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
      // Access can change while a turn is running. Discard even an observation
      // from an in-flight response that used the previous connection.
      if (text.trim().split(/\s+/)[0] === "/providers") resetInput();
      options.finishActivity(activity);
    }
  }

  return command;
}
