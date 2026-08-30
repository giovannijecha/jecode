// Reassembling an OpenAI Responses reply from its event stream.
//
// Unlike Anthropic, this stream ends with the whole finished response in
// `response.completed`, so there is nothing to rebuild: the deltas drive the
// display, and the final event is taken as authoritative. Items collected
// along the way are only a fallback for a stream that ends without it.
export async function assembleOpenAI(events, onStream) {
    const items = [];
    let completed;
    let refusal = false;
    for await (const raw of events) {
        const event = raw;
        switch (event.type) {
            case "response.output_text.delta":
                if (typeof event.delta === "string")
                    onStream?.({ kind: "text", text: event.delta });
                break;
            case "response.refusal.delta":
                if (typeof event.delta === "string") {
                    onStream?.({ kind: "text", text: `${refusal ? "" : "[refused] "}${event.delta}` });
                    refusal = true;
                }
                break;
            case "response.reasoning_summary_text.delta":
                if (typeof event.delta === "string")
                    onStream?.({ kind: "thinking", text: event.delta });
                break;
            case "response.output_item.done":
                if (event.item !== undefined)
                    items.push(event.item);
                break;
            case "response.completed":
            case "response.incomplete":
                if (event.response !== undefined)
                    completed = event.response;
                break;
            case "response.failed": {
                const response = event.response;
                throw new Error(`openai stream error: ${response?.error?.message ?? "unspecified"}`);
            }
            case "error":
                throw new Error(`openai stream error: ${event.error?.message ?? event.message ?? "unspecified"}`);
            default:
                break;
        }
    }
    return completed ?? { output: items };
}
