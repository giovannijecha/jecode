// Small projections of the live session used by the shell and footer.

import type { ControllerOptions } from "../controller.ts";
import { credentialSource } from "../credentials.ts";
import { providerFailure } from "../provider-errors.ts";
import type { Session } from "../session.ts";
import type { Block, NoticeBlock } from "./blocks.ts";
import type { FooterInfo } from "./components/footer.ts";

export function controllerOptions(session: Session): ControllerOptions {
  return {
    provider: session.provider,
    tools: session.tools,
    model: session.model,
    system: session.system,
    maxTokens: session.config.maxTokens,
    effort: session.config.effort,
    maxSteps: session.config.maxSteps,
    toolContext: { root: session.config.root },
  };
}

export function footerInfo(session: Session, workspace = session.config.root): FooterInfo {
  return {
    workspace,
    model: session.model || "no model",
    effort: session.config.effort,
  };
}

/** One transcript event for a real turn failure, including actionable auth guidance. */
export function turnFailure(session: Session, error: Error, aborted: boolean): NoticeBlock {
  if (aborted) return { kind: "notice", text: "[interrupted]", tone: "warn" };

  let text = providerFailure(session.provider, error);
  if (/\b401\b/.test(text)) {
    const source = credentialSource(session.provider.keyVar);
    text += source === "environment"
      ? ` · update ${session.provider.keyVar} in the environment and restart`
      : " · check credentials with /settings";
  }
  return { kind: "notice", text, tone: "error" };
}

export function toggleDetails(blocks: Block[]): Block | undefined {
  for (let index = blocks.length - 1; index >= 0; index--) {
    const block = blocks[index];
    if (block?.kind === "tool" && (block.body?.length ?? 0) > 0) {
      block.expanded = block.expanded !== true;
      return block;
    }
    if (block?.kind === "reasoning") {
      block.expanded = block.expanded !== true;
      return block;
    }
  }
  return undefined;
}
