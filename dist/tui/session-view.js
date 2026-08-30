// Small projections of the live session used by the shell and footer.
import { credentialSource } from "../credentials.js";
import { providerFailure } from "../provider-errors.js";
export function controllerOptions(session) {
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
export function footerInfo(session, workspace = session.config.root) {
    return {
        workspace,
        model: session.model || "no model",
        effort: session.config.effort,
    };
}
/** One transcript event for a real turn failure, including actionable auth guidance. */
export function turnFailure(session, error, aborted) {
    if (aborted)
        return { kind: "notice", text: "[interrupted]", tone: "warn" };
    let text = providerFailure(session.provider, error);
    if (/\b401\b/.test(text)) {
        const source = credentialSource(session.provider.keyVar);
        text += source === "environment"
            ? ` · update ${session.provider.keyVar} in the environment and restart`
            : " · check credentials with /settings";
    }
    return { kind: "notice", text, tone: "error" };
}
export function toggleDetails(blocks) {
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
