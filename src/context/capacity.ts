// Resolve model context metadata without making it a requirement for a turn.

import type { ModelContextWindow, Provider } from "../types.ts";
import type { ContextPolicy } from "./policy.ts";
import { policyForContextWindow } from "./policy.ts";

export type ResolveContextPolicyOptions = Readonly<{
  provider: Provider;
  model: string;
  compactionPercent: number;
  signal?: AbortSignal;
  onStatus?: (status: string) => void;
}>;

export async function resolveContextPolicy(
  options: ResolveContextPolicyOptions,
): Promise<ContextPolicy> {
  const context = await optionalContextWindow(options);
  return policyForContextWindow(context, options.compactionPercent);
}

async function optionalContextWindow(
  options: ResolveContextPolicyOptions,
): Promise<ModelContextWindow | undefined> {
  if (options.provider.contextWindow === undefined) return undefined;
  try {
    return await options.provider.contextWindow(
      options.model,
      options.signal,
      options.onStatus,
    );
  } catch (error) {
    if (options.signal?.aborted === true) throw error;
    return undefined;
  }
}
