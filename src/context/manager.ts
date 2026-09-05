// One automatic-compaction lifecycle, independent of terminal and persistence.

import type { ContextRequest } from "../controller.ts";
import type { ConversationRequestIdentity, Message, Provider, ToolSpec, Usage } from "../types.ts";
import { automaticCompactionKey } from "./automatic.ts";
import type { AutomaticCompactionGate } from "./automatic.ts";
import { compactContext } from "./compactor.ts";
import type { CompactionDiagnostic } from "./diagnostics.ts";
import type { ContextAnchor } from "./projection.ts";
import { isContextOverflow } from "./policy.ts";

export type ContextDiagnostic = CompactionDiagnostic;

type ContextManagerOptions = Readonly<{
  provider: Provider;
  model: string;
  effort: string;
  system: string;
  tools: readonly ToolSpec[];
  maxOutputTokens: number;
  historyStart: number;
  nodeId(): number;
  gate: AutomaticCompactionGate;
  identity: ConversationRequestIdentity;
  signal?: AbortSignal;
  onStatus(active: boolean): void;
  onUsage(usage: Usage): void;
  onDiagnostic?(event: ContextDiagnostic): void;
}>;

export function contextManager(options: ContextManagerOptions) {
  let anchor: ContextAnchor | undefined;
  return {
    get anchor(): ContextAnchor | undefined {
      return anchor;
    },
    async compact(
      canonical: readonly Message[],
      projected: readonly Message[],
      request: ContextRequest,
    ): Promise<readonly Message[] | undefined> {
      const force = request.reason === "overflow";
      if (force && (request.error === undefined || !isContextOverflow(request.error))) return undefined;
      if (!force && request.inputTokens < request.policy.triggerTokens) return undefined;
      const nodeId = options.nodeId();
      const attempt = {
        key: automaticCompactionKey(options.provider.id, options.model, nodeId, canonical.length),
        scope: `${options.provider.id}\0${options.model}\0${options.identity.conversationId}\0` +
          `${request.policy.windowTokens}\0${request.policy.triggerTokens}`,
        reason: request.reason,
        inputTokens: request.inputTokens,
        retryGrowthTokens: Math.max(1_024, Math.min(
          request.policy.minimumPrefixTokens,
          request.policy.requestLimitTokens - 256 - request.inputTokens,
        )),
      };
      if (!options.gate.allows(attempt)) return undefined;
      let started = false;
      const result = await compactContext({
        reason: request.reason,
        onDiagnostic: options.onDiagnostic,
        provider: options.provider,
        model: options.model,
        effort: options.effort,
        context: projected,
        turn: canonical.slice(options.historyStart),
        nodeId,
        coveredMessages: anchor?.messageCount ?? 0,
        lastInputTokens: 0,
        estimatedInputTokens: request.inputTokens,
        force,
        policy: request.policy,
        signal: options.signal,
        requestEnvelope: {
          system: options.system,
          tools: options.tools,
          maxOutputTokens: options.maxOutputTokens,
        },
        requestIdentity: options.identity,
        onBegin() {
          started = true;
          options.onStatus(true);
        },
        onEnd() { options.onStatus(false); },
        onUsage: options.onUsage,
      });
      if (result === undefined) {
        if (started && options.signal?.aborted !== true) options.gate.failed(attempt);
        return undefined;
      }
      options.gate.succeeded(attempt);
      anchor = result.anchor;
      return result.messages;
    },
  };
}
