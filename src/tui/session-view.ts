// Small projections of the live session used by the shell and footer.

import type { ControllerOptions } from "../controller.ts";
import { credentialSource } from "../credentials.ts";
import { providerFailure } from "../provider-errors.ts";
import type { Session } from "../session.ts";
import type { SteeringSource } from "../steering.ts";
import type { Block, NoticeBlock } from "./blocks.ts";
import type { FooterInfo } from "./components/footer.ts";

export function controllerOptions(
  session: Session,
  contextPolicy: ControllerOptions["contextPolicy"],
  tools: Session["tools"] = session.tools,
  steering?: SteeringSource,
): ControllerOptions {
  return {
    provider: session.provider,
    tools,
    model: session.model,
    system: session.system,
    maxTokens: session.config.maxTokens,
    contextPolicy,
    effort: session.config.effort,
    maxModelRequests: session.config.maxModelRequests,
    toolContext: { root: session.config.root },
    ...(steering === undefined ? {} : { steering }),
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
    const auth = session.provider.auth;
    if (auth.kind === "oauth") {
      text += ` · reconnect ${auth.label} in /providers`;
    } else {
      const source = credentialSource(auth.keyVar);
      text += source === "environment"
        ? ` · update ${auth.keyVar} in the environment and restart`
        : " · check access with /providers";
    }
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
