// Explicit compaction of the selected durable leaf.
//
// The canonical messages and transcript remain untouched. Only the active
// leaf receives a new branch-local context anchor, using the same provider
// policy and summarizer as automatic compaction.

import type { Session } from "../session.ts";
import { recordAuxiliaryUsage } from "../usage.ts";
import { toolSpecs } from "../tools/index.ts";
import { resolveContextPolicy } from "./capacity.ts";
import { compactContext } from "./compactor.ts";
import { estimateRequestInputTokensResponsive } from "./budget.ts";
import { estimateTokensResponsive, planCompaction } from "./policy.ts";
import { projectToolResultsNewest, toolResultProjectionBudget } from "./request-projection.ts";
import { requestIdentityForSession } from "../request-identity.ts";

const MIN_PREFIX_TOKENS = 512;

export type ManualCompactionResult = "compacted" | "unchanged";

export type ManualCompactionOptions = Readonly<{
  signal?: AbortSignal;
  onStatus?(status?: string): void;
}>;

export async function compactSession(
  session: Session,
  options: ManualCompactionOptions = {},
): Promise<ManualCompactionResult> {
  const active = session.conversation.activeNode;
  if (active === undefined) return "unchanged";
  const context = session.conversation.contextHistory;
  if (session.conversation.nodes.some((node) => node.parentId === active.id)) {
    throw new Error("continue this branch before compacting");
  }

  options.onStatus?.("Checking context");
  const policy = await resolveContextPolicy({
    provider: session.provider,
    model: session.model,
    compactionPercent: session.config.compactionPercent,
    signal: options.signal,
    onStatus: (status) => options.onStatus?.(status),
  });
  const specs = toolSpecs(session.tools);
  const estimatedInputTokens = await estimateRequestInputTokensResponsive({
    system: session.system,
    messages: projectToolResultsNewest(context, toolResultProjectionBudget(policy)).messages,
    tools: specs,
  }, options.signal);
  if (estimatedInputTokens < MIN_PREFIX_TOKENS) {
    options.onStatus?.();
    return "unchanged";
  }
  const coveredMessages = active.context?.throughNodeId === active.id
    ? active.context.messageCount
    : 0;
  const plan = await planCompaction(
    context,
    active.messages,
    coveredMessages,
    session.usage.lastInputTokens,
    true,
    policy,
    estimatedInputTokens,
    options.signal,
  );
  if (
    plan === undefined ||
    await estimateTokensResponsive(plan.prefix, options.signal) < MIN_PREFIX_TOKENS
  ) {
    options.onStatus?.();
    return "unchanged";
  }

  const result = await compactContext({
    provider: session.provider,
    model: session.model,
    effort: session.config.effort,
    context,
    turn: active.messages,
    nodeId: active.id,
    coveredMessages,
    lastInputTokens: session.usage.lastInputTokens,
    estimatedInputTokens,
    precomputedPlan: plan,
    signal: options.signal,
    force: true,
    failLoudly: true,
    policy,
    requestEnvelope: {
      system: session.system,
      tools: specs,
      maxOutputTokens: session.config.maxTokens,
    },
    requestIdentity: requestIdentityForSession(session),
    onBegin: () => options.onStatus?.("Compacting"),
    onEnd: () => options.onStatus?.(),
  });
  if (result === undefined) throw new Error("context could not be compacted");
  if (result.usage !== undefined) recordAuxiliaryUsage(session.usage, result.usage);

  const next = session.conversation.commit({
    nodeId: active.id,
    parentId: active.parentId,
    createdAt: active.createdAt,
    identity: active.identity,
    messages: active.messages,
    blocks: active.blocks,
    context: result.anchor,
    ...(active.failure === undefined ? {} : { failure: active.failure }),
  }, active.settlement);
  await session.persistence?.checkpoint(next);
  session.conversation = next;
  return "compacted";
}
