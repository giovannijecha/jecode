// Local input estimates anchored only to an unchanged, provider-measured prefix.

import { createHash } from "node:crypto";
import type { Message, Provider, RequestInput } from "../types.ts";
import { estimateRequestInputTokensResponsive } from "./budget.ts";

export type InputMeasurement = Readonly<{
  epoch: number;
  inputTokens: number;
  estimatedTokens: number;
  source: "estimate" | "provider-prefix";
  envelope: string;
  messages: readonly string[];
}>;

export type InputMeter = Readonly<{
  measure(input: RequestInput, signal?: AbortSignal): Promise<InputMeasurement>;
  observe(measurement: InputMeasurement, reportedInputTokens: number | undefined): void;
  reset(): void;
}>;

/** A response's usage is evidence about that exact request, never later history. */
export function inputMeter(provider: Provider): InputMeter {
  let baseline: Readonly<{ measurement: InputMeasurement; reported: number }> | undefined;
  let epoch = 0;
  return {
    async measure(input, signal) {
      const measuredEpoch = epoch;
      const estimatedTokens = await measureInput(provider, input, signal);
      const envelope = fingerprint({
        provider: provider.id,
        model: input.model,
        effort: input.effort,
        system: input.system,
        tools: input.tools,
      });
      const messages: string[] = [];
      for (const message of input.messages) {
        signal?.throwIfAborted();
        // Including raw and usage is deliberately conservative: even a change
        // to an opaque reserve must invalidate the old provider observation.
        messages.push(fingerprint(message));
        await new Promise<void>((resolve) => setImmediate(resolve));
      }
      const previous = baseline?.measurement;
      const anchored = measuredEpoch === epoch && previous !== undefined && previous.envelope === envelope &&
        previous.messages.length <= messages.length &&
        previous.messages.every((value, index) => value === messages[index]);
      const inputTokens = anchored && baseline !== undefined
        ? baseline.reported + Math.max(0, estimatedTokens - baseline.measurement.estimatedTokens)
        : estimatedTokens;
      if (!anchored && measuredEpoch === epoch) baseline = undefined;
      return Object.freeze({
        epoch: measuredEpoch,
        inputTokens,
        estimatedTokens,
        source: anchored ? "provider-prefix" : "estimate",
        envelope,
        messages: Object.freeze(messages),
      });
    },
    observe(measurement, reported) {
      if (measurement.epoch !== epoch || reported === undefined || !Number.isSafeInteger(reported) || reported <= 0) return;
      baseline = { measurement, reported };
    },
    reset() {
      epoch++;
      baseline = undefined;
    },
  };
}

export async function measureInput(
  provider: Provider,
  input: RequestInput,
  signal?: AbortSignal,
): Promise<number> {
  signal?.throwIfAborted();
  const tokens = provider.measureInput === undefined
    ? await estimateRequestInputTokensResponsive(input, signal)
    : await provider.measureInput(input, signal);
  signal?.throwIfAborted();
  if (!Number.isSafeInteger(tokens) || tokens <= 0) {
    throw new Error("provider returned an invalid input token estimate");
  }
  return tokens;
}

/** Retention must include opaque replay reserves, excluding the fixed envelope. */
export async function messageCounter(
  provider: Provider,
  model: string,
  effort: string,
  signal?: AbortSignal,
): Promise<(messages: readonly Message[], signal?: AbortSignal) => Promise<number>> {
  const input = { model, effort, system: "", messages: [], tools: [] };
  const overhead = await measureInput(provider, input, signal);
  return async (messages, currentSignal = signal) => Math.max(1,
    await measureInput(provider, { ...input, messages: [...messages] }, currentSignal) - overhead);
}

function fingerprint(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}
