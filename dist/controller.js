// The single controller.
//
// One loop, in dialogue with the user. It never delegates: no subagent, no
// worker, no orchestrator split, no second loop. When a task is too big, this
// loop iterates. That constraint is implemented here, not left as a product claim.
import { isToolCall } from "./types.js";
import { findTool, runTool, toolSpecs } from "./tools/index.js";
/**
 * Run one user turn to completion: keep exchanging with the model until it
 * stops asking for tools. `history` is mutated in place, so an aborted turn
 * still leaves the conversation in a consistent state.
 */
export async function runTurn(history, options, events, signal) {
    const specs = toolSpecs(options.tools);
    for (let step = 0; step < options.maxSteps; step++) {
        events.onStep?.(step + 1, options.maxSteps);
        // The message is displayed as it streams; what comes back here is the
        // assembled version, which exists to be appended to the history.
        const assistant = await options.provider.send({
            model: options.model,
            system: options.system,
            messages: history,
            tools: specs,
            maxTokens: options.maxTokens,
            effort: options.effort,
            signal,
            onStream: (event) => events.onStream(event),
            onStatus: (status) => events.onStatus?.(status),
        });
        history.push(assistant);
        if (assistant.usage !== undefined)
            events.onUsage?.(assistant.usage);
        const calls = assistant.content.filter(isToolCall);
        if (calls.length === 0)
            return; // the model is done — hand back to the user
        // Calls run one after another because approval prompts serialise anyway,
        // but every result from this step goes back in a SINGLE message. Splitting
        // them teaches the model to stop batching its calls.
        const results = [];
        for (let index = 0; index < calls.length; index++) {
            const call = calls[index];
            events.onToolProgress?.(index + 1, calls.length);
            const preview = await look(call, options);
            events.onToolCall(call, preview);
            const { result, summary } = await settle(call, options, events, signal, preview);
            events.onToolResult(call, result, summary);
            results.push(result);
        }
        history.push({ role: "user", content: results });
    }
    throw new Error(`gave up after ${options.maxSteps} steps without finishing (raise --max-steps)`);
}
async function settle(call, options, events, signal, preview) {
    const tool = findTool(options.tools, call.name);
    if (tool === undefined) {
        return refuse(call, `no such tool: ${call.name}`, "unknown tool");
    }
    if (tool.dangerous && !(await events.approve(call))) {
        return refuse(call, "the user declined this call — ask them how to proceed", "declined");
    }
    return runTool(tool, call, { ...options.toolContext, signal, preview });
}
function refuse(call, reason, summary) {
    return {
        result: { kind: "tool_result", id: call.id, output: reason, isError: true },
        summary,
    };
}
/**
 * What the call would change, asked of the tool that would change it.
 *
 * Read-only and best-effort by construction: a preview that throws — a missing
 * file, a match that is not there — is simply no preview. Nothing about the
 * turn depends on it, because it exists for the user, not for the model.
 */
async function look(call, options) {
    const tool = findTool(options.tools, call.name);
    if (tool?.preview === undefined)
        return undefined;
    try {
        return await tool.preview(call.input, options.toolContext);
    }
    catch {
        return undefined;
    }
}
