// Complete ordinary input; bounded tool projection is an emergency fallback.

import type { Message, RequestInput } from "../types.ts";
import type { InputMeasurement, InputMeter } from "./measurement.ts";
import type { ContextPolicy } from "./policy.ts";
import { MIN_REQUEST_OUTPUT_TOKENS } from "./policy.ts";
import { projectToolResultsNewest, toolResultProjectionBudget } from "./request-projection.ts";

export type PreparedInput = Readonly<{
  messages: Message[];
  measurement: InputMeasurement;
  clippedResults: number;
}>;

/** Called after optional semantic compaction, immediately before sending. */
export async function fitRequestInput(
  input: RequestInput,
  meter: InputMeter,
  policy: ContextPolicy,
  measurement: InputMeasurement,
  signal?: AbortSignal,
): Promise<PreparedInput> {
  const limit = policy.requestLimitTokens - MIN_REQUEST_OUTPUT_TOKENS;
  if (measurement.inputTokens <= limit) {
    return { messages: [...input.messages], measurement, clippedResults: 0 };
  }

  let budget = toolResultProjectionBudget(policy);
  for (;;) {
    signal?.throwIfAborted();
    const projection = projectToolResultsNewest(input.messages, budget);
    const next = await meter.measure({ ...input, messages: projection.messages }, signal);
    if (next.inputTokens <= limit || budget === 0 || projection.clippedResults === 0) {
      return { messages: projection.messages, measurement: next, clippedResults: projection.clippedResults };
    }
    // At most logarithmically many bounded attempts, then the existing request
    // guard reports an irreducible prompt/schema instead of a provider call.
    budget = budget < 256 ? 0 : Math.floor(budget / 2);
  }
}
