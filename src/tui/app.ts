// The TUI shell: keys in, frames out, one turn at a time.
//
// Everything mutable about a session lives in `state` here. The view is a pure
// function of it, so the only job of a key handler is to change state and ask
// for a repaint — never to draw.

import type { Session } from "../session.ts";
import type { Activity, ActivityKind } from "./activity.ts";
import { activityStatus, begin } from "./activity.ts";
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
import { resumePicker } from "./resume.ts";

const FRAME_MS = 16;
const MOTION_FRAME_MS = 40;
const ACTIVITY_REFRESH_MS = 1_000;
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
  shutdownSignal?: AbortSignal;
};

export async function runApp(
  session: Session,
  transcriptRoot: string,
  environment: AppEnvironment = {},
): Promise<void> {
  const terminal = environment.screen ?? realScreen;
  const paint = environment.paint ?? painter();
  const keys = decoder({ ctrlBackspaceIsBs: process.env["WT_SESSION"] !== undefined });
  const transcript = transcriptRenderer();
  const workspace = await workspaceLabel(session.config.root);

  const state = appState();
  state.blocks.push(...session.conversation.transcript);
  state.committedNodeId = session.conversation.activeNodeId;

  const permissions = sessionPermissions(session.tools);

  let closed: (() => void) | undefined;
  let frameTimer: NodeJS.Timeout | undefined;
  let motionTimer: NodeJS.Timeout | undefined;
  let activityTimer: NodeJS.Timeout | undefined;
  let escapeTimer: NodeJS.Timeout | undefined;
  let stopResize = (): void => {};
  let stopInput = (): void => {};
  let stopShutdown = (): void => {};
  let failure: { error: unknown } | undefined;
  const activeWorkflows = new Set<Promise<void>>();
  // Timers outlive the teardown they were scheduled before. Painting after the
  // terminal has been handed back would write escapes into the user's shell.
  let live = true;
  const done = new Promise<void>((resolve) => {
    closed = resolve;
  });

  const guard = (action: () => void): void => {
    try {
      action();
    } catch (error) {
      fail(error);
    }
  };

  const view = () => {
    const now = Date.now();
    const activity = state.command !== undefined && state.open === undefined && state.approval === undefined
      ? state.command : state.activity ?? state.command;
    return {
      blocks: state.blocks,
      editor: state.editor,
      scroll: state.scroll,
      unseen: state.unseen,
      pal: session.palette,
      footer: state.turnFooter === undefined ? footerInfo(session, workspace) : { ...state.turnFooter, workspace },
      status: activity === undefined
        ? undefined
        : activityStatus(activity, now),
      turnActive: state.activity !== undefined,
      steering: state.command === undefined ? state.steering : undefined,
      feedback: state.feedback,
      readiness: turnBlocker(session),
      now,
      reducedMotion: session.config.reducedMotion,
      modal: overlay.shown(state.approval ?? state.open),
      menu: completionOptions(state.completing),
      menuIndex: state.completing?.index,
    };
  };

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
    if (frame.transcriptPending) render();
    if (frame.transcriptAnimating && motionTimer === undefined) {
      motionTimer = setTimeout(() => guard(() => {
        motionTimer = undefined;
        render();
      }), Math.max(0, MOTION_FRAME_MS - FRAME_MS));
    } else if (!frame.transcriptAnimating && motionTimer !== undefined) {
      clearTimeout(motionTimer);
      motionTimer = undefined;
    }
  };

  // Streaming produces a token at a time; painting at that rate is wasted
  // work, so repaints coalesce onto one frame.
  const render = (block?: Block): void => {
    if (block !== undefined) transcript.invalidate(block);
    if (live && frameTimer === undefined) {
      frameTimer = setTimeout(() => guard(draw), FRAME_MS);
    }
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

  const replaceTranscript = (): void => {
    state.blocks.splice(0, state.blocks.length, ...session.conversation.transcript);
    state.scroll = 0;
    state.follow = true;
    state.unseen = 0;
    state.lastMaxScroll = 0;
    transcript.invalidate();
    paint.invalidate();
    render();
  };

  function quit(): void {
    if (!live) return;
    live = false;
    safely(stopInput);
    safely(stopResize);
    safely(stopShutdown);
    if (activityTimer !== undefined) clearInterval(activityTimer);
    if (frameTimer !== undefined) clearTimeout(frameTimer);
    if (motionTimer !== undefined) clearTimeout(motionTimer);
    if (escapeTimer !== undefined) clearTimeout(escapeTimer);
    safely(() => feedback.close());
    safely(() => terminal.leave());
    closed?.();
  }

  function safely(action: () => void): void {
    try {
      action();
    } catch (error) {
      failure ??= { error };
    }
  }

  function fail(error: unknown): void {
    failure ??= { error };
    state.open = overlay.cancel(state.open);
    state.approval = overlay.cancel(state.approval);
    state.activity?.control.abort(error);
    state.command?.control.abort(error);
    quit();
  }

  function track(work: Promise<void>): Promise<void> {
    const tracked = work
      .catch((error: unknown) => fail(error))
      .finally(() => {
        activeWorkflows.delete(tracked);
      });
    activeWorkflows.add(tracked);
    return tracked;
  }

  function requestQuit(reason: unknown = new Error("interrupted")): void {
    state.open = overlay.cancel(state.open);
    state.approval = overlay.cancel(state.approval);
    if (state.activity === undefined && state.command === undefined) {
      quit();
      return;
    }
    state.closeWhenIdle = true;
    state.activity?.control.abort(reason);
    state.command?.control.abort(reason);
  }

  function startActivity(kind: ActivityKind, label: string): Activity | undefined {
    if (state.closeWhenIdle) return undefined;
    if (state.command !== undefined || (kind === "turn" && state.activity !== undefined)) return undefined;
    const activity = begin(kind, label);
    if (kind === "turn") state.activity = activity;
    else state.command = activity;
    activityTimer ??= setInterval(
      () => guard(() => {
        let activeTool: Block | undefined;
        for (let index = state.blocks.length - 1; index >= 0; index--) {
          const block = state.blocks[index];
          if (block?.kind !== "tool" || block.tone !== "pending" || block.startedAt === undefined) continue;
          activeTool = block;
          break;
        }
        render(activeTool);
      }),
      ACTIVITY_REFRESH_MS,
    );
    render();
    return activity;
  }

  function finishActivity(activity: Activity): void {
    if (state.activity === activity) {
      state.activity = undefined;
      state.turnFooter = undefined;
      state.approval = overlay.cancel(state.approval);
    } else if (state.command === activity) {
      state.command = undefined;
      state.open = overlay.cancel(state.open);
    } else return;
    const idle = state.activity === undefined && state.command === undefined;
    if (idle && activityTimer !== undefined) {
      clearInterval(activityTimer);
      activityTimer = undefined;
    }
    if (state.closeWhenIdle && idle) quit();
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
    replaceTranscript,
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
    actions: {
      command: (text) => track(actions.command(text)),
      turn: (text) => track(actions.turn(text)),
      steer: (text) => actions.steer(text),
    },
    live: () => live,
    quit,
    requestQuit,
    scrollBy,
    invalidate: () => paint.invalidate(),
    transcriptChanged: (block) => transcript.invalidate(block),
  });

  const resumeAtLaunch = session.resume === undefined
    ? undefined
    : track(openResumedSession(session.resume));

  async function openResumedSession(launch: NonNullable<Session["resume"]>): Promise<void> {
    while (live) {
      const index = await new Promise<number | undefined>((resolve) => {
        state.open = { picker: resumePicker(launch.candidates, session.palette), settle: resolve };
        render();
      });
      if (index === undefined) {
        quit();
        return;
      }
      const candidate = launch.candidates[index];
      if (candidate === undefined) continue;
      const activity = startActivity("command", "Opening session");
      if (activity === undefined) return;
      try {
        await launch.open(candidate.id);
        replaceTranscript();
        state.committedNodeId = session.conversation.activeNodeId;
        state.past.length = 0;
        finishActivity(activity);
        return;
      } catch (error) {
        feedback.show({ text: (error as Error).message, tone: "error", timeoutMs: 6_000 });
        finishActivity(activity);
      }
    }
  }

  try {
    const shutdownSignal = environment.shutdownSignal;
    const onShutdown = (): void => requestQuit(shutdownSignal?.reason);
    if (shutdownSignal?.aborted === true) return;
    shutdownSignal?.addEventListener("abort", onShutdown, { once: true });
    stopShutdown = () => shutdownSignal?.removeEventListener("abort", onShutdown);
    terminal.enter(session.config.reducedMotion);
    stopResize = terminal.onResize(() => guard(() => {
      paint.invalidate();
      render();
    }));
    stopInput = terminal.onInput((chunk) => guard(() => {
      if (escapeTimer !== undefined) clearTimeout(escapeTimer);
      for (const key of keys.push(chunk)) {
        if (!live) break;
        input.handle(key);
      }
      if (!live) return;
      escapeTimer = setTimeout(() => guard(() => {
        escapeTimer = undefined;
        if (!live) return;
        for (const key of keys.flush()) input.handle(key);
        if (live) render();
      }), ESCAPE_MS);
      render();
    }));

    draw();
    if (resumeAtLaunch !== undefined) await Promise.race([resumeAtLaunch, done]);
    if (live) await done;
    if (failure !== undefined) throw failure.error;
  } finally {
    try {
      quit();
      await Promise.allSettled([...activeWorkflows]);
    } finally {
      await session.persistence?.close();
    }
  }
}
