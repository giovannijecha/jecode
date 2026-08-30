// Translation between the normalized vocabulary and the OpenAI Responses wire
// shape: a flat `input` list where tool calls and their outputs are top-level
// items keyed by `call_id`, rather than blocks nested inside a message.
// The Responses API has no `xhigh`; collapse it onto the nearest level rather
// than passing through a value it will reject.
export function normalizeEffort(effort) {
    return effort === "xhigh" || effort === "max" ? "high" : effort;
}
export function toWireTool(tool) {
    return {
        type: "function",
        name: tool.name,
        description: tool.description,
        parameters: tool.input,
    };
}
export function toWireItems(message) {
    if (message.rawFrom === "openai" && Array.isArray(message.raw)) {
        return message.raw;
    }
    const items = [];
    const texts = [];
    for (const block of message.content) {
        if (block.kind === "text") {
            texts.push(block.text);
        }
        else if (block.kind === "tool_call") {
            items.push({
                type: "function_call",
                call_id: block.id,
                name: block.name,
                arguments: JSON.stringify(block.input),
            });
        }
        else {
            items.push({ type: "function_call_output", call_id: block.id, output: block.output });
        }
    }
    if (texts.length > 0) {
        const type = message.role === "assistant" ? "output_text" : "input_text";
        items.unshift({ role: message.role, content: [{ type, text: texts.join("\n") }] });
    }
    return items;
}
export function stopNotice(data) {
    const reason = data.incomplete_details?.reason;
    if (reason === undefined)
        return undefined;
    return reason === "max_output_tokens"
        ? "[truncated: hit max_output_tokens — raise --max-tokens]"
        : `[incomplete: ${reason}]`;
}
export function fromWireResponse(data) {
    const raw = Array.isArray(data.output) ? data.output : [];
    const content = [];
    for (const entry of raw) {
        const item = entry;
        if (item.type === "message" && Array.isArray(item.content)) {
            for (const part of item.content) {
                const piece = part;
                if (piece.type === "output_text" && typeof piece.text === "string") {
                    content.push({ kind: "text", text: piece.text });
                }
                else if (piece.type === "refusal" && typeof piece.refusal === "string") {
                    content.push({ kind: "text", text: `[refused] ${piece.refusal}` });
                }
            }
        }
        else if (item.type === "function_call" &&
            typeof item.call_id === "string" &&
            typeof item.name === "string") {
            content.push({
                kind: "tool_call",
                id: item.call_id,
                name: item.name,
                input: parseArguments(item.arguments),
            });
        }
        // reasoning items and anything future: carried in `raw` only.
    }
    const notice = stopNotice(data);
    if (notice !== undefined)
        content.push({ kind: "text", text: notice });
    return { role: "assistant", content, raw, rawFrom: "openai", usage: normalizeUsage(data) };
}
function normalizeUsage(data) {
    const usage = data.usage;
    if (usage === undefined || usage === null)
        return undefined;
    return {
        inputTokens: usage.input_tokens ?? 0,
        outputTokens: usage.output_tokens ?? 0,
        cachedInputTokens: usage.input_tokens_details?.cached_tokens ?? 0,
        cacheWriteInputTokens: usage.input_tokens_details?.cache_write_tokens ?? 0,
        reasoningTokens: usage.output_tokens_details?.reasoning_tokens ?? 0,
    };
}
// Arguments arrive as a JSON string and models vary in how they escape it, so
// this always goes through a real parse — never string matching.
function parseArguments(text) {
    if (text === undefined || text === "")
        return {};
    try {
        const parsed = JSON.parse(text);
        return typeof parsed === "object" && parsed !== null
            ? parsed
            : {};
    }
    catch {
        return {};
    }
}
