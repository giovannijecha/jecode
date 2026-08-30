// Semantic transcript blocks routed to small, owned production components.
import { renderAnswer, renderReasoning, renderUser } from "./components/messages.js";
import { renderList, renderNotice } from "./components/misc.js";
import { renderTool } from "./components/tool.js";
export function render(block, width, pal) {
    switch (block.kind) {
        case "user":
            return renderUser(block, width, pal);
        case "answer":
            return renderAnswer(block, width, pal);
        case "reasoning":
            return renderReasoning(block, width, pal);
        case "tool":
            return renderTool(block, width, pal);
        case "notice":
            return renderNotice(block, width, pal);
        case "list":
            return renderList(block, width, pal);
    }
}
export function renderAll(blocks, width, pal) {
    return blocks.flatMap((block) => render(block, width, pal));
}
