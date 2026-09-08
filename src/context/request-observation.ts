// Observe the provider boundary, including failures before a response can be saved.
import type { Message, Provider, SendRequest, TransportObservation } from "../types.ts";
import type { InputMeasurement } from "./measurement.ts";
import type { ContextPolicy } from "./policy.ts";
import { publishDiagnostic } from "./diagnostics.ts";
import { networkErrorCode } from "../providers/network-diagnostic.ts";
import type { NetworkCode } from "../providers/network-diagnostic.ts";
import { transportFailureDiagnostic } from "../providers/transport-error.ts";
import type { TransportFailureDiagnostic } from "../providers/transport-error.ts";

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
  let firstTextMs: number | undefined;
  let firstThinkingMs: number | undefined;
  let transport: TransportObservation | undefined;
  let response: Message | undefined;
  let networkCode: NetworkCode | undefined;
  let transportFailure: TransportFailureDiagnostic = {};
  let outcome: "completed" | "failed" | "cancelled" = "failed";
  try {
    response = await provider.send({ ...request, onTransport(event) {
      transport = event;
      request.onTransport?.(event);
    }, onStream(event) {
      firstEventMs ??= Math.round(performance.now() - started);
      if (event.kind === "text") firstTextMs ??= Math.round(performance.now() - started);
      if (event.kind === "thinking") firstThinkingMs ??= Math.round(performance.now() - started);
      request.onStream?.(event);
    } });
    request.signal?.throwIfAborted();
    outcome = "completed";
    return response;
  } catch (error) {
    if (request.signal?.aborted) outcome = "cancelled";
    else {
      networkCode = networkErrorCode(error);
      transportFailure = transportFailureDiagnostic(error);
    }
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
      ...(firstTextMs === undefined ? {} : { firstTextMs }),
      ...(firstThinkingMs === undefined ? {} : { firstThinkingMs }),
      ...transport,
      ...(networkCode === undefined ? {} : { networkCode }),
      ...transportFailure,
      ...(response?.usage === undefined ? {} : {
        reportedInputTokens: response.usage.inputTokens, outputTokens: response.usage.outputTokens,
        cachedInputTokens: response.usage.cachedInputTokens, cacheWriteInputTokens: response.usage.cacheWriteInputTokens,
        reasoningTokens: response.usage.reasoningTokens,
      }),
    });
  }
}
