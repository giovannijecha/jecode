// Keyboard and pointer intent for the TUI shell.
import { activate as activateCompletion, move as moveCompletion, selected as selectedCompletion, } from "./complete.js";
import * as edit from "./editor.js";
import { turnBlocker } from "./feedback.js";
import { applyKey } from "./input.js";
import * as overlay from "./overlay.js";
import { toggleDetails } from "./session-view.js";
const SCROLL_STEP = 8;
const WHEEL_STEP = 3;
export function appInput(options) {
    const { session, state, feedback, actions } = options;
    function handle(key) {
        if (!options.live())
            return;
        // The wheel only changes what is visible. It never answers an open prompt.
        if (key.name === "pointer") {
            const wheel = key.pointer?.wheel;
            if (wheel !== undefined)
                options.scrollBy(wheel === "up" ? WHEEL_STEP : -WHEEL_STEP);
            return;
        }
        if (state.feedback !== undefined)
            feedback.dismiss();
        if (state.open !== undefined) {
            const outcome = overlay.handle(state.open, key);
            state.open = outcome.open;
            if (outcome.abort === true)
                state.activity?.control.abort(new Error("interrupted"));
            if (outcome.quit === true)
                options.requestQuit();
            return;
        }
        if (key.ctrl && key.name === "c") {
            if (state.activity !== undefined)
                state.activity.control.abort(new Error("interrupted"));
            else
                options.quit();
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
                    if (completed !== undefined)
                        state.editor = edit.of(completed);
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
        if (key.ctrl && key.name === "o") {
            const changed = toggleDetails(state.blocks);
            if (changed !== undefined)
                options.transcriptChanged(changed);
            return;
        }
        const edited = applyKey(state.editor, key);
        if (edited !== undefined) {
            state.editor = edited;
            state.completing = activateCompletion(edited.text);
        }
    }
    function recall(step) {
        if (state.past.length === 0)
            return;
        state.completing = undefined;
        if (state.recall === -1) {
            if (step > 0)
                return;
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
        state.editor = edit.of(state.past[state.recall]);
    }
    function submit() {
        const text = state.editor.text.trim();
        if (text === "" || state.activity !== undefined)
            return;
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
        if (isCommand)
            void actions.command(text);
        else
            void actions.turn(text);
    }
    return { handle };
}
