// Responses can spend minutes reasoning between application events. Give both
// transports the same bounded progress window; socket activity is not progress.
import { TransportError } from "./transport-error.ts";

export const MODEL_PROGRESS_TIMEOUT_MS = 300_000;

export function modelProgressTimeout(): TransportError {
  return new TransportError("progress-timeout",
    `Model stream made no model progress for ${MODEL_PROGRESS_TIMEOUT_MS}ms`);
}
