// The normalized vocabulary the controller speaks. Deliberately smaller than
// any vendor's wire format: text, a tool call, a tool result. Providers
// translate to and from this; the controller never sees a vendor shape.
export function isToolCall(block) {
    return block.kind === "tool_call";
}
export function isText(block) {
    return block.kind === "text";
}
