// The TUI shell: keys in, frames out, one turn at a time.
//
// Everything mutable about a session lives in `state` here. The view is a pure
// function of it, so the only job of a key handler is to change state and ask
// for a repaint — never to draw.

import type { Session } from "../session.ts";
import type { Activity, ActivityKind } from "./activity.ts";
import { begin } from "./activity.ts";
import * as overlay from "./overlay.ts";
import type { Block, NoticeBlock } from "./blocks.ts";
import { options as completionOptions } from "./complete.ts";
import { decoder } from "./keys.ts";
import type { Painter } from "./frame.ts";
import { painter } from "./frame.ts";
import { commandFeedback, feedbackController, turnBlocker } from "./feedback.ts";
import * as realScreen from "./screen.ts";
import type { Size } from "./screen.ts";
import { compose } from "./view.ts";
import { preserveOffset } from "./scroll.ts";
import { footerInfo } from "./session-view.ts";
import { workspaceLabel } from "./workspace.ts";
import { transcriptRenderer } from "./transcript-view.ts";
import { appState } from "./app-state.ts";
import { appInput } from "./app-input.ts";
import { appWorkflows } from "./app-workflows.ts";
import { sessionPermissions } from "../permissions.ts";

const FRAME_MS = 16;
const SPIN_MS = 80;
/** How long a lone escape waits to prove it is not the start of a sequence. */
const ESCAPE_MS = 25;

export type AppScreen = {
  size(): Size;
  enter(reducedMotion?: boolean): void;
  leave(): void;
  setReducedMotion(reducedMotion: boolean): void;
  onResize(handler: () => void): () => void;
  onInput(handler: (chunk: string) => void): () => void;
};

export type AppEnvironment = {
  screen?: AppScreen;
  paint?: Painter;
};

export async function runApp(
  session: Session,
  transcriptRoot: string,
  environment: AppEnvironment = {},
): Promise<void> {
  const terminal = environment.screen ?? realScreen;
  const paint = environment.paint ?? painter();
  const keys = decoder();
  const transcript = transcriptRenderer();
  const workspace = await workspaceLabel(session.config.root);

  const state = appState();

  const permissions = sessionPermissions(session.tools, session.config.autoApprove);

  let closed: (() => void) | undefined;
  let frameTimer: NodeJS.Timeout | undefined;
  let spinTimer: NodeJS.Timeout | undefined;
  let escapeTimer: NodeJS.Timeout | undefined;
  let stopResize = (): void => {};
  let stopInput = (): void => {};
  // Timers outlive the teardown they were scheduled before. Painting after the
  // terminal has been handed back would write escapes into the user's shell.
  let live = true;
  const done = new Promise<void>((resolve) => {
    closed = resolve;
  });

  const view = () => ({
    blocks: state.blocks,
    editor: state.editor,
    scroll: state.scroll,
    unseen: state.unseen,
    pal: session.palette,
    footer: footerInfo(session, workspace),
    status: state.status,
    feedback: state.feedback,
    readiness: turnBlocker(session),
    spin: state.spin,
    reducedMotion: session.config.reducedMotion,
    now: Date.now(),
    modal: overlay.shown(state.open),
    menu: completionOptions(state.completing),
    menuIndex: state.completing?.index,
  });

  const draw = (): void => {
    frameTimer = undefined;
    if (!live) return;
    let frame = compose(view(), terminal.size(), transcript);
    // Preserve the exact rows being read while streaming grows the transcript.
    // `scroll` is measured from the bottom, so new rows increase that offset.
    if (!state.follow && frame.maxScroll > state.lastMaxScroll) {
      state.scroll = preserveOffset(
        state.scroll,
        state.follow,
        state.lastMaxScroll,
        frame.maxScroll,
      );
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
  const render = (block?: Block): void => {
    if (block !== undefined) transcript.invalidate(block);
    if (live && frameTimer === undefined) frameTimer = setTimeout(draw, FRAME_MS);
  };

  const feedback = feedbackController((next) => {
    state.feedback = next;
    render();
  });

  const emit = (block: Block): void => {
    state.blocks.push(block);
    if (state.follow) state.scroll = 0;
    else state.unseen++;
  };

  const commandNotice = (notice: NoticeBlock): void => {
    feedback.show(commandFeedback(notice));
  };

  const scrollBy = (amount: number): void => {
    state.scroll = Math.max(0, state.scroll + amount);
    state.follow = state.scroll === 0;
    if (state.follow) state.unseen = 0;
  };

  function quit(): void {
    if (!live) return;
    live = false;
    stopInput();
    stopResize();
    if (spinTimer !== undefined) clearInterval(spinTimer);
    if (frameTimer !== undefined) clearTimeout(frameTimer);
    if (escapeTimer !== undefined) clearTimeout(escapeTimer);
    feedback.close();
    terminal.leave();
    closed?.();
  }

  function requestQuit(): void {
    const activity = state.activity;
    if (activity === undefined) {
      quit();
      return;
    }
    state.closeWhenIdle = true;
    state.open = overlay.cancel(state.open);
    activity.control.abort(new Error("interrupted"));
  }

  function startActivity(kind: ActivityKind, label: string): Activity | undefined {
    if (state.activity !== undefined) return undefined;
    const activity = begin(kind, label);
    state.activity = activity;
    state.status = label;
    spinTimer = setInterval(() => {
      if (!session.config.reducedMotion) state.spin++;
      let activeTool: Block | undefined;
      for (let index = state.blocks.length - 1; index >= 0; index--) {
        const block = state.blocks[index];
        if (block?.kind !== "tool" || block.tone !== "pending" || block.startedAt === undefined) continue;
        activeTool = block;
        break;
      }
      render(activeTool);
    }, session.config.reducedMotion ? 1_000 : SPIN_MS);
    render();
    return activity;
  }

  function finishActivity(activity: Activity): void {
    if (state.activity !== activity) return;
    if (spinTimer !== undefined) clearInterval(spinTimer);
    spinTimer = undefined;
    state.activity = undefined;
    state.status = undefined;
    state.open = overlay.cancel(state.open);
    if (state.closeWhenIdle) quit();
    else render();
  }

  const actions = appWorkflows({
    session,
    transcriptRoot,
    state,
    permissions,
    feedback,
    emit,
    commandNotice,
    render,
    refreshSettings: () => {
      terminal.setReducedMotion(session.config.reducedMotion);
      paint.invalidate();
      transcript.invalidate();
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
    if (escapeTimer !== undefined) clearTimeout(escapeTimer);
    for (const key of keys.push(chunk)) {
      if (!live) break;
      input.handle(key);
    }
    if (!live) return;
    escapeTimer = setTimeout(() => {
      escapeTimer = undefined;
      if (!live) return;
      for (const key of keys.flush()) input.handle(key);
      if (live) render();
    }, ESCAPE_MS);
    render();
  });

  draw();
  await done;
}
