// One bounded ingress for text that can become a user prompt.

import { MAX_TEXT_CODE_UNITS } from "./text-boundary.ts";

export const MAX_PROMPT_CODE_UNITS = MAX_TEXT_CODE_UNITS;
export const PROMPT_LIMIT_MESSAGE =
  `Prompt cannot exceed ${MAX_PROMPT_CODE_UNITS.toLocaleString("en-US")} UTF-16 code units`;

export class PromptLimitError extends Error {
  constructor() {
    super(PROMPT_LIMIT_MESSAGE);
    this.name = "PromptLimitError";
  }
}

export function assertPromptLength(length: number): void {
  if (length > MAX_PROMPT_CODE_UNITS) throw new PromptLimitError();
}

export function assertPromptAppend(current: number, added: number): void {
  if (added > MAX_PROMPT_CODE_UNITS - current) throw new PromptLimitError();
}
