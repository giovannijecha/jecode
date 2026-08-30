// Provider-neutral token accounting for one in-memory session.
export function emptyUsage() {
    return {
        requests: 0,
        lastInputTokens: 0,
        inputTokens: 0,
        outputTokens: 0,
        cachedInputTokens: 0,
        cacheWriteInputTokens: 0,
        reasoningTokens: 0,
    };
}
export function recordUsage(total, next) {
    total.requests += 1;
    total.lastInputTokens = next.inputTokens;
    total.inputTokens += next.inputTokens;
    total.outputTokens += next.outputTokens;
    total.cachedInputTokens += next.cachedInputTokens;
    total.cacheWriteInputTokens += next.cacheWriteInputTokens;
    total.reasoningTokens += next.reasoningTokens;
}
export function formatTokens(value) {
    if (value < 1_000)
        return String(value);
    if (value < 10_000)
        return `${(value / 1_000).toFixed(1)}k`;
    if (value < 1_000_000)
        return `${Math.round(value / 1_000)}k`;
    return `${(value / 1_000_000).toFixed(1)}m`;
}
