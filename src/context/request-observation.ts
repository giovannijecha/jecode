// Observe the provider boundary, including failures before a response can be saved.
import type { Message, Provider, SendRequest } from "../types.ts";
import type { InputMeasurement } from "./measurement.ts";
import type { ContextPolicy } from "./policy.ts";
import { publishDiagnostic } from "./diagnostics.ts";

export async function observePreparation<T>(
  policy: ContextPolicy,
  reason: "budget" | "overflow",
  signal: AbortSignal | undefined,
  prepare: () => Promise<T>,
): Promise<T> {
  const started = performance.now();
  try { return await prepare(); }
  catch (error) {
    publishDiagnostic({ kind: "preparation", reason, outcome: signal?.aborted ? "cancelled" : "failed",
      elapsedMs: Math.round(performance.now() - started), windowTokens: policy.windowTokens,
      triggerTokens: policy.triggerTokens, requestLimitTokens: policy.requestLimitTokens });
    throw error;
  }
}

export async function sendObserved(
  provider: Provider,
  request: SendRequest,
  measurement: InputMeasurement,
  policy: ContextPolicy,
  preparationMs: number,
  clippedResults: number,
): Promise<Message> {
  const started = performance.now();
  let firstEventMs: number | undefined;
  let response: Message | undefined;
  let outcome: "completed" | "failed" | "cancelled" = "failed";
  try {
    response = await provider.send({ ...request, onStream(event) {
      firstEventMs ??= Math.round(performance.now() - started);
      request.onStream?.(event);
    } });
    request.signal?.throwIfAborted();
    outcome = "completed";
    return response;
  } catch (error) {
    if (request.signal?.aborted) outcome = "cancelled";
    throw error;
  } finally {
    publishDiagnostic({
      kind: "request", source: measurement.source, outcome,
      tokenization: provider.inputTokenization?.(request.model) ?? "heuristic",
      estimatedTokens: measurement.estimatedTokens, inputTokens: measurement.inputTokens,
      windowTokens: policy.windowTokens, triggerTokens: policy.triggerTokens,
      requestLimitTokens: policy.requestLimitTokens, outputBudgetTokens: request.maxTokens,
      preparationMs: Math.round(preparationMs), providerMs: Math.round(performance.now() - started),
      clippedResults,
      ...(firstEventMs === undefined ? {} : { firstEventMs }),
      ...(response?.usage === undefined ? {} : { reportedInputTokens: response.usage.inputTokens }),
    });
  }
}
