// Reassembling a Chat Completions reply from its event stream.
//
// Nothing arrives finished here: text accumulates, and each tool call is
// spread across chunks keyed by `index` — id and name usually in the first,
// arguments as JSON fragments after it.
import { addBounded, MAX_TOOL_ARGUMENT_CHARS } from "./stream-limits.js";
export async function assembleOllama(events, onStream) {
    const calls = new Map();
    let toolArgumentChars = 0;
    let content = "";
    let finishReason;
    let usage;
    for await (const raw of events) {
        const event = raw;
        if (event.error !== undefined) {
            const message = typeof event.error === "string" ? event.error : event.error.message;
            throw new Error(`ollama stream error: ${message ?? "unspecified"}`);
        }
        if (event.usage !== undefined)
            usage = event.usage;
        const choice = event.choices?.[0];
        if (choice === undefined)
            continue;
        if (typeof choice.finish_reason === "string")
            finishReason = choice.finish_reason;
        const delta = choice.delta;
        if (delta === undefined)
            continue;
        // Reasoning has no standardized field name across the models Ollama
        // serves, so both spellings in circulation are accepted.
        const reasoning = delta.reasoning ?? delta.reasoning_content;
        if (typeof reasoning === "string" && reasoning !== "") {
            onStream?.({ kind: "thinking", text: reasoning });
        }
        if (typeof delta.content === "string" && delta.content !== "") {
            content += delta.content;
            onStream?.({ kind: "text", text: delta.content });
        }
        for (const part of delta.tool_calls ?? []) {
            const index = typeof part.index === "number" ? part.index : 0;
            const call = calls.get(index) ?? { id: "", name: "", args: "" };
            if (typeof part.id === "string" && part.id !== "")
                call.id = part.id;
            if (typeof part.function?.name === "string" && part.function.name !== "") {
                call.name = part.function.name;
            }
            if (typeof part.function?.arguments === "string") {
                toolArgumentChars = addBounded(toolArgumentChars, part.function.arguments.length, MAX_TOOL_ARGUMENT_CHARS, "streamed tool arguments");
                call.args += part.function.arguments;
            }
            calls.set(index, call);
        }
    }
    const toolCalls = [...calls.entries()]
        .sort(([a], [b]) => a - b)
        // Not every server sends an id, and the loop needs one to pair the result
        // back to its call.
        .map(([index, call]) => (call.id === "" ? { ...call, id: `call_${index}` } : call));
    return { content, toolCalls, finishReason, usage };
}
