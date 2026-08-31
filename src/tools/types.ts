import type { ToolSpec } from "../types.ts";

export type ToolContext = {
  /** The directory the agent is confined to. */
  root: string;
  signal?: AbortSignal;
  /** The exact state the user approved, when this call presented a preview. */
  preview?: ToolPreview;
  /** Current bounded, credential-redacted output while a command is running. */
  onOutput?(output: string): void;
};

export type ToolOutput = {
  /** What the model reads. */
  output: string;
  /**
   * What the user sees at the right of the tool's row — the size of what
   * happened, not a story about it: "214 lines", "18 entries", "exit 0".
   */
  summary?: string;
};

/**
 * What a call would change: the file as it is, and the file as it would be.
 *
 * Only the tool can answer this. `edit_file` carries both halves of its change
 * in its arguments, but `write_file` carries only the new contents — nothing
 * outside the tool knows whether that is a new file or a replacement, and
 * "twenty-two added lines" is a very different question from "twenty-two lines
 * replacing these nineteen".
 */
export type ToolPreview = { before: string; after: string };

export type Tool = ToolSpec & {
  /** Whether the controller must ask the user before every call. */
  readonly dangerous: boolean;
  /**
   * What the user is shown before being asked to approve the call.
   *
   * Never load-bearing: it must not write, and a failure means no preview
   * rather than a failed turn. The call itself is still what decides.
   */
  preview?(args: Record<string, unknown>, ctx: ToolContext): Promise<ToolPreview | undefined>;
  run(args: Record<string, unknown>, ctx: ToolContext): Promise<ToolOutput>;
};
