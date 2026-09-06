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
import { measureInput, messageCounter } from "./measurement.ts";
import { planCompaction } from "./policy.ts";
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
  const estimatedInputTokens = await measureInput(session.provider, {
    model: session.model,
    effort: session.config.effort,
    system: session.system,
    messages: context,
    tools: specs,
  }, options.signal);
  if (estimatedInputTokens < MIN_PREFIX_TOKENS) {
    options.onStatus?.();
    return "unchanged";
  }
  const coveredMessages = active.context?.throughNodeId === active.id
    ? active.context.messageCount
    : 0;
  const countMessages = await messageCounter(session.provider, session.model, session.config.effort, options.signal);
  const plan = await planCompaction(
    context,
    active.messages,
    coveredMessages,
    0,
    true,
    policy,
    estimatedInputTokens,
    options.signal,
    countMessages,
  );
  if (
    plan === undefined ||
    await countMessages(plan.prefix, options.signal) < MIN_PREFIX_TOKENS
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
    lastInputTokens: 0,
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
    onUsage: (usage) => recordAuxiliaryUsage(session.usage, usage),
  });
  if (result === undefined) throw new Error("context could not be compacted");

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
