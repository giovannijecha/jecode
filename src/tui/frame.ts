// Painting a frame by writing only the rows that changed.
//
// The whole frame is recomposed on every render — that is what makes the view
// a pure function of the state. What must not happen on every render is
// writing it: a full repaint at streaming speed flickers and floods the pipe.
// So the painter keeps the last frame and emits the difference.

import { CSI } from "../ui/render.ts";
import { CURSOR, SYNC, write, outputReady, onDrain } from "./screen.ts";

/** Zero-based position within the frame. */
export type Cursor = { row: number; col: number };

export type Painter = {
  paint(rows: readonly string[], cursor?: Cursor): void;
  /** Forget the last frame, so the next paint writes every row. */
  invalidate(): void;
  /** Request a fresh frame when the output can accept another write. */
  onReady?(handler: () => void): () => void;
};

export type FrameOutput = {
  write(text: string): void;
  ready(): boolean;
  onReady(handler: () => void): () => void;
};

export function painter(output: FrameOutput = { write, ready: outputReady, onReady: onDrain }): Painter {
  let previous: string[] = [];

  return {
    paint(rows: readonly string[], cursor?: Cursor): void {
      // The last accepted frame remains the diff base. The host redraws current
      // state on drain, so stale intermediate frames never form another queue.
      if (!output.ready()) return;
      let out = SYNC.begin + CURSOR.hide;
      const height = Math.max(rows.length, previous.length);

      for (let i = 0; i < height; i++) {
        const next = rows[i] ?? "";
        if (next === previous[i]) continue;
        out += `${CSI}${i + 1};1H${CSI}2K${next}`;
      }

      if (cursor !== undefined) {
        out += `${CSI}${cursor.row + 1};${cursor.col + 1}H${CURSOR.show}`;
      }

      previous = rows.slice();
      // One write, so a terminal that ignores the synchronization hint still
      // gets the frame as a single arrival rather than a row at a time.
      output.write(out + SYNC.end);
    },

    invalidate(): void {
      previous = [];
    },
    onReady: (handler) => output.onReady(handler),
  };
}
