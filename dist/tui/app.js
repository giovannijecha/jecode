// The TUI shell: keys in, frames out, one turn at a time.
//
// Everything mutable about a session lives in `state` here. The view is a pure
// function of it, so the only job of a key handler is to change state and ask
// for a repaint — never to draw.
import { begin, elapsed } from "./activity.js";
import * as overlay from "./overlay.js";
import { options as completionOptions } from "./complete.js";
import { decoder } from "./keys.js";
import { painter } from "./frame.js";
import { commandFeedback, feedbackController, turnBlocker } from "./feedback.js";
import * as realScreen from "./screen.js";
import { compose } from "./view.js";
import { preserveOffset } from "./scroll.js";
import { footerInfo } from "./session-view.js";
import { workspaceLabel } from "./workspace.js";
import { transcriptRenderer } from "./transcript-view.js";
import { appState } from "./app-state.js";
import { appInput } from "./app-input.js";
import { appWorkflows } from "./app-workflows.js";
const FRAME_MS = 16;
const SPIN_MS = 80;
/** How long a lone escape waits to prove it is not the start of a sequence. */
const ESCAPE_MS = 25;
export async function runApp(session, transcriptRoot, environment = {}) {
    const terminal = environment.screen ?? realScreen;
    const paint = environment.paint ?? painter();
    const keys = decoder();
    const transcript = transcriptRenderer();
    const workspace = await workspaceLabel(session.config.root);
    const state = appState();
    // Tools the user said "always" to. It lives for the window and dies with it:
    // a permission granted once, in one conversation, is not a setting.
    const allowed = new Map();
    let closed;
    let frameTimer;
    let spinTimer;
    let stopResize = () => { };
    let stopInput = () => { };
    // Timers outlive the teardown they were scheduled before. Painting after the
    // terminal has been handed back would write escapes into the user's shell.
    let live = true;
    const done = new Promise((resolve) => {
        closed = resolve;
    });
    const view = () => ({
        blocks: state.blocks,
        editor: state.editor,
        scroll: state.scroll,
        unseen: state.unseen,
        pal: session.palette,
        footer: footerInfo(session, workspace),
        status: state.status === undefined || state.activity === undefined
            ? state.status
            : `${state.status} · ${elapsed(state.activity)}`,
        feedback: state.feedback,
        readiness: turnBlocker(session),
        spin: state.spin,
        reducedMotion: session.config.reducedMotion,
        modal: overlay.shown(state.open),
        menu: completionOptions(state.completing),
        menuIndex: state.completing?.index,
    });
    const draw = () => {
        frameTimer = undefined;
        if (!live)
            return;
        let frame = compose(view(), terminal.size(), transcript);
        // Preserve the exact rows being read while streaming grows the transcript.
        // `scroll` is measured from the bottom, so new rows increase that offset.
        if (!state.follow && frame.maxScroll > state.lastMaxScroll) {
            state.scroll = preserveOffset(state.scroll, state.follow, state.lastMaxScroll, frame.maxScroll);
            frame = compose(view(), terminal.size(), transcript);
        }
        // Composing is what discovers how far there is to scroll, so the clamp
        // lands here rather than in every place that moves the viewport.
        state.scroll = Math.min(state.scroll, frame.maxScroll);
        if (state.scroll === 0) {
            state.follow = true;
            state.unseen = 0;
        }
        state.lastMaxScroll = frame.maxScroll;
        paint.paint(frame.rows, frame.cursor);
    };
    // Streaming produces a token at a time; painting at that rate is wasted
    // work, so repaints coalesce onto one frame.
    const render = (block) => {
        if (block !== undefined)
            transcript.invalidate(block);
        if (live && frameTimer === undefined)
            frameTimer = setTimeout(draw, FRAME_MS);
    };
    const feedback = feedbackController((next) => {
        state.feedback = next;
        render();
    });
    const emit = (block) => {
        state.blocks.push(block);
        if (state.follow)
            state.scroll = 0;
        else
            state.unseen++;
    };
    const commandOutput = (block) => {
        const next = commandFeedback(block);
        if (next === undefined)
            emit(block);
        else
            feedback.show(next);
    };
    const scrollBy = (amount) => {
        state.scroll = Math.max(0, state.scroll + amount);
        state.follow = state.scroll === 0;
        if (state.follow)
            state.unseen = 0;
    };
    function quit() {
        if (!live)
            return;
        live = false;
        stopInput();
        stopResize();
        if (spinTimer !== undefined)
            clearInterval(spinTimer);
        if (frameTimer !== undefined)
            clearTimeout(frameTimer);
        feedback.close();
        terminal.leave();
        closed?.();
    }
    function requestQuit() {
        const activity = state.activity;
        if (activity === undefined) {
            quit();
            return;
        }
        state.closeWhenIdle = true;
        state.open = overlay.cancel(state.open);
        activity.control.abort(new Error("interrupted"));
    }
    function startActivity(kind, label) {
        if (state.activity !== undefined)
            return undefined;
        const activity = begin(kind, label);
        state.activity = activity;
        state.status = label;
        spinTimer = setInterval(() => {
            if (!session.config.reducedMotion)
                state.spin++;
            render();
        }, session.config.reducedMotion ? 1_000 : SPIN_MS);
        render();
        return activity;
    }
    function finishActivity(activity) {
        if (state.activity !== activity)
            return;
        if (spinTimer !== undefined)
            clearInterval(spinTimer);
        spinTimer = undefined;
        state.activity = undefined;
        state.status = undefined;
        state.open = overlay.cancel(state.open);
        if (state.closeWhenIdle)
            quit();
        else
            render();
    }
    const actions = appWorkflows({
        session,
        transcriptRoot,
        state,
        allowed,
        feedback,
        emit,
        commandOutput,
        render,
        refreshSettings: () => {
            terminal.setReducedMotion(session.config.reducedMotion);
            paint.invalidate();
            render();
        },
        startActivity,
        finishActivity,
    });
    const input = appInput({
        session,
        state,
        feedback,
        actions,
        live: () => live,
        quit,
        requestQuit,
        scrollBy,
        invalidate: () => paint.invalidate(),
        transcriptChanged: (block) => transcript.invalidate(block),
    });
    terminal.enter(session.config.reducedMotion);
    stopResize = terminal.onResize(() => {
        paint.invalidate();
        draw();
    });
    stopInput = terminal.onInput((chunk) => {
        for (const key of keys.push(chunk))
            input.handle(key);
        setTimeout(() => {
            for (const key of keys.flush())
                input.handle(key);
            render();
        }, ESCAPE_MS);
        render();
    });
    draw();
    await done;
}
