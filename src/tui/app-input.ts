// Keyboard and pointer intent for the TUI shell.

import type { Session } from "../session.ts";
import type { AppState } from "./app-state.ts";
import type { Block } from "./blocks.ts";
import {
  activate as activateCompletion,
  move as moveCompletion,
  selected as selectedCompletion,
} from "./complete.ts";
import * as edit from "./editor.ts";
import type { FeedbackController } from "./feedback.ts";
import { turnBlocker } from "./feedback.ts";
import { applyKey } from "./input.ts";
import type { Key } from "./keys.ts";
import * as overlay from "./overlay.ts";
import { toggleDetails } from "./session-view.ts";

const SCROLL_STEP = 8;
const WHEEL_STEP = 3;

export type AppActions = {
  command(text: string): Promise<void>;
  turn(text: string): Promise<void>;
};

type InputOptions = {
  session: Session;
  state: AppState;
  feedback: FeedbackController;
  actions: AppActions;
  live(): boolean;
  quit(): void;
  requestQuit(): void;
  scrollBy(amount: number): void;
  invalidate(): void;
  transcriptChanged(block: Block): void;
};

export function appInput(options: InputOptions): { handle(key: Key): void } {
  const { session, state, feedback, actions } = options;

  function handle(key: Key): void {
    if (!options.live()) return;

    // The wheel only changes what is visible. It never answers an open prompt.
    if (key.name === "pointer") {
      const wheel = key.pointer?.wheel;
      if (wheel !== undefined) options.scrollBy(wheel === "up" ? WHEEL_STEP : -WHEEL_STEP);
      return;
    }

    if (state.feedback !== undefined) feedback.dismiss();

    // Detail expansion remains available while an approval is open. A large
    // diff may be compacted, but the user must be able to inspect it before
    // answering the permission prompt.
    if (key.ctrl && key.name === "o") {
      const changed = toggleDetails(state.blocks);
      if (changed !== undefined) options.transcriptChanged(changed);
      return;
    }

    if (state.open !== undefined) {
      const outcome = overlay.handle(state.open, key);
      state.open = outcome.open;
      if (outcome.abort === true) state.activity?.control.abort(new Error("interrupted"));
      if (outcome.quit === true) options.requestQuit();
      return;
    }

    if (key.ctrl && key.name === "c") {
      if (state.activity !== undefined) state.activity.control.abort(new Error("interrupted"));
      else options.quit();
      return;
    }

    if (key.ctrl && key.name === "d" && state.editor.text === "") {
      options.requestQuit();
      return;
    }

    switch (key.name) {
      case "escape":
        if (state.completing !== undefined) {
          state.completing = undefined;
          return;
        }
        state.activity?.control.abort(new Error("interrupted"));
        return;
      case "enter": {
        if (state.completing !== undefined) {
          const completed = selectedCompletion(state.completing);
          if (completed !== undefined) state.editor = edit.of(completed);
          state.completing = undefined;
        }
        submit();
        return;
      }
      case "tab": {
        const completion = state.completing ?? activateCompletion(state.editor.text);
        const completed = completion === undefined ? undefined : selectedCompletion(completion);
        if (completed !== undefined) {
          state.editor = edit.of(completed);
          state.completing = undefined;
        }
        return;
      }
      case "up":
        if (state.completing !== undefined) {
          state.completing = moveCompletion(state.completing, -1);
          return;
        }
        recall(-1);
        return;
      case "down":
        if (state.completing !== undefined) {
          state.completing = moveCompletion(state.completing, 1);
          return;
        }
        recall(1);
        return;
      case "pageup":
        options.scrollBy(SCROLL_STEP);
        return;
      case "pagedown":
        options.scrollBy(-SCROLL_STEP);
        return;
      default:
        break;
    }

    if (key.ctrl && key.name === "l") {
      options.invalidate();
      return;
    }

    const edited = applyKey(state.editor, key);
    if (edited !== undefined) {
      state.editor = edited;
      state.completing = activateCompletion(edited.text);
    }
  }

  function recall(step: number): void {
    if (state.past.length === 0) return;
    state.completing = undefined;

    if (state.recall === -1) {
      if (step > 0) return;
      state.draft = state.editor.text;
      state.recall = state.past.length;
    }

    const next = state.recall + step;
    if (next >= state.past.length) {
      state.recall = -1;
      state.editor = edit.of(state.draft);
      return;
    }

    state.recall = Math.max(0, next);
    state.editor = edit.of(state.past[state.recall] as string);
  }

  function submit(): void {
    const text = state.editor.text.trim();
    if (text === "" || state.activity !== undefined) return;

    const isCommand = text.startsWith("/");
    if (!isCommand) {
      const blocker = turnBlocker(session);
      if (blocker !== undefined) {
        feedback.show(blocker);
        return;
      }
    }

    state.editor = edit.EMPTY;
    state.recall = -1;
    state.draft = "";
    state.completing = undefined;
    state.past.push(text);
    state.scroll = 0;
    state.follow = true;
    state.unseen = 0;

    if (isCommand) void actions.command(text);
    else void actions.turn(text);
  }

  return { handle };
}
