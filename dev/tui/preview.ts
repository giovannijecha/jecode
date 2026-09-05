// Production input and rendering with an inert, memory-only session.

import { ConversationTree } from "../../src/conversation.ts";
import type { Session } from "../../src/session.ts";
import { steeringInbox } from "../../src/steering.ts";
import { emptyUsage } from "../../src/usage.ts";
import { begin } from "../../src/tui/activity.ts";
import { appInput } from "../../src/tui/app-input.ts";
import { appState } from "../../src/tui/app-state.ts";
import type { Block } from "../../src/tui/blocks.ts";
import { render } from "../../src/tui/blocks.ts";
import { activate, options as completionOptions } from "../../src/tui/complete.ts";
import * as edit from "../../src/tui/editor.ts";
import type { Key } from "../../src/tui/keys.ts";
import type { Picker } from "../../src/tui/picker.ts";
import { edge, selected } from "../../src/tui/picker.ts";
import * as overlay from "../../src/tui/overlay.ts";
import { preserveOffset } from "../../src/tui/scroll.ts";
import { transcriptRenderer } from "../../src/tui/transcript-view.ts";
import type { Size } from "../../src/tui/screen.ts";
import { compose } from "../../src/tui/view.ts";
import type { View } from "../../src/tui/view.ts";

export type PreviewActions = {
  command(text: string): void;
  pick?(index: number, picker: Picker): void;
};

export function createPreview(initial: View, actions: PreviewActions) {
  let source = initial;
  let sourceCount = initial.blocks.length;
  let interrupted = false;
  let exited = false;
  let rendered = false;
  let selection: { index: number; picker: Picker } | undefined;
  const state = appState();
  state.blocks = structuredClone([...initial.blocks]);
  state.editor = initial.editor;
  state.scroll = initial.scroll;
  state.follow = initial.scroll === 0;
  state.feedback = initial.feedback;
  state.completing = initial.menu === undefined ? undefined : activate(initial.editor.text);
  if (state.completing !== undefined) state.completing.index = initial.menuIndex ?? 0;
  if (initial.status !== undefined) state.activity = begin("turn", "Preview", 0);
  state.steering = initial.steering;
  const inbox = steeringInbox((pending, accepting) => {
    state.steering = accepting ? pending : undefined;
  });
  const transcript = transcriptRenderer(render, () => source.now ?? 0);
  const feedback = {
    show(value: NonNullable<View["feedback"]>) { state.feedback = value; },
    dismiss() { state.feedback = undefined; },
    close() { state.feedback = undefined; },
  };

  const modal = initial.modal;
  if (modal?.kind === "pick") {
    state.open = {
      picker: selected(modal.picker) === undefined ? edge(modal.picker, "home") : modal.picker,
      settle(index?: number) {
        if (index !== undefined && state.open !== undefined && "picker" in state.open) {
          selection = { index, picker: state.open.picker };
        }
      },
    };
  } else if (modal?.kind === "type") {
    state.open = { field: modal.field, settle() {} };
  } else if (modal?.kind === "help") {
    state.open = { help: true, settle() {} };
  }

  const input = appInput({
    session: inertSession(initial), state, feedback,
    actions: {
      async command(text) { actions.command(text); },
      async turn(text) {
        state.blocks.push({ kind: "user", text });
        state.blocks.push({ kind: "answer", text: "Preview input received." });
      },
      steer(text) { return inbox.offer(text); },
    },
    live: () => !exited,
    quit() { exited = true; },
    requestQuit() { exited = true; },
    scrollBy(amount) {
      state.scroll = Math.max(0, state.scroll + amount);
      state.follow = state.scroll === 0;
    },
    invalidate() { transcript.invalidate(); },
    transcriptChanged(block) { transcript.invalidate(block); },
  });

  function view(): View {
    return {
      ...source, blocks: state.blocks, editor: state.editor, scroll: state.scroll,
      modal: overlay.shown(state.open), menu: completionOptions(state.completing),
      menuIndex: state.completing?.index, feedback: state.feedback,
      status: interrupted ? undefined : source.status,
      steering: interrupted ? undefined : state.steering,
    };
  }

  function interrupt(): void {
    interrupted = true;
    state.activity = undefined;
    for (const block of state.blocks) {
      if (block.kind === "tool" && block.tone === "pending") {
        block.tone = "fail";
        block.right = "interrupted";
        if (block.startedAt !== undefined) block.durationMs = Math.max(0, (source.now ?? 0) - block.startedAt);
        delete block.startedAt;
        transcript.invalidate(block);
      }
      if (block.kind === "reasoning" && block.live) {
        block.live = false;
        transcript.invalidate(block);
      }
    }
    returnGuidance();
  }

  function returnGuidance(): void {
    const queued = inbox.close();
    if (queued.length > 0) state.editor = edit.of([...queued, state.editor.text].filter(Boolean).join("\n"));
  }

  return {
    view,
    get interrupted() { return interrupted; },
    get exited() { return exited; },
    handle(key: Key) {
      input.handle(key);
      if (state.activity?.control.signal.aborted) interrupt();
      const picked = selection;
      selection = undefined;
      if (picked !== undefined) actions.pick?.(picked.index, picked.picker);
    },
    sync(next: View) {
      if (interrupted) return;
      source = next;
      const retained = state.blocks.slice(0, sourceCount);
      const appended = state.blocks.slice(sourceCount);
      let structureChanged = retained.length !== next.blocks.length;
      const updated = next.blocks.map((fresh, index) => {
        const old = retained[index];
        if (old === undefined || !sameBlock(old, fresh)) {
          structureChanged = true;
          const added = structuredClone(fresh);
          transcript.invalidate(added);
          return added;
        }
        // Input owns detail expansion; advancing the fixture clock keeps it.
        const candidate = { ...fresh };
        if ((old.kind === "tool" || old.kind === "reasoning") &&
            (candidate.kind === "tool" || candidate.kind === "reasoning")) {
          candidate.expanded = old.expanded;
        }
        if (JSON.stringify(old) !== JSON.stringify(candidate)) {
          for (const key of Object.keys(old)) delete (old as unknown as Record<string, unknown>)[key];
          Object.assign(old, candidate);
          transcript.invalidate(old);
        }
        return old;
      });
      state.blocks.splice(0, state.blocks.length, ...updated, ...appended);
      if (structureChanged) transcript.invalidate();
      sourceCount = next.blocks.length;
      if (next.status === undefined && state.activity !== undefined) {
        state.activity = undefined;
        returnGuidance();
      }
    },
    style(pal: View["pal"], reducedMotion: boolean) {
      source = { ...source, pal, reducedMotion };
      transcript.invalidate();
    },
    invalidate() { transcript.invalidate(); },
    render(size: Size) {
      let frame = compose(view(), size, transcript);
      if (rendered && !state.follow && frame.maxScroll > state.lastMaxScroll) {
        state.scroll = preserveOffset(state.scroll, false, state.lastMaxScroll, frame.maxScroll);
        frame = compose(view(), size, transcript);
      }
      state.scroll = Math.min(state.scroll, frame.maxScroll);
      state.lastMaxScroll = frame.maxScroll;
      rendered = true;
      if (state.scroll === 0) state.follow = true;
      return frame;
    },
    close() { exited = true; inbox.close(); feedback.close(); },
  };
}

function sameBlock(left: Block, right: Block): boolean {
  if (left.kind !== right.kind) return false;
  if (left.kind !== "tool" || right.kind !== "tool") return true;
  return left.name === right.name && left.target === right.target;
}

function inertSession(view: View): Session {
  const model = view.footer.model;
  return {
    config: {
      providerId: "anthropic", model, root: "/lab", effort: "high",
      reducedMotion: false, maxTokens: 4096, compactionPercent: 85,
      autoApprove: false, ephemeral: true,
    },
    provider: {
      id: "anthropic", defaultModel: model,
      auth: { kind: "api-key", keyVar: "ANTHROPIC_API_KEY" },
      blocked: () => view.readiness === undefined ? undefined : "ANTHROPIC_API_KEY is missing",
      async models() { return []; },
      async send() { throw new Error("the TUI lab cannot call a provider"); },
    },
    model, palette: view.pal, tools: [], system: "",
    conversation: ConversationTree.empty(), usage: emptyUsage(),
  };
}
