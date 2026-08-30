// The tool registry, and the one place a tool actually gets run.
import { editFile, listDir, readFile, writeFile } from "./fs.js";
import { runCommand } from "./shell.js";
import { findFiles, searchText } from "./search.js";
export function builtinTools() {
    return [readFile, listDir, findFiles, searchText, editFile, writeFile, runCommand];
}
export function findTool(tools, name) {
    return tools.find((tool) => tool.name === name);
}
/** Strip the executable half — providers only ever see the declaration. */
export function toolSpecs(tools) {
    return tools.map((tool) => ({
        name: tool.name,
        description: tool.description,
        input: tool.input,
    }));
}
// A tool failure is a result, not an exception: the model reads the message
// and gets another turn to fix its call. Only an aborted turn propagates.
export async function runTool(tool, call, ctx) {
    try {
        const { output, summary } = await tool.run(call.input, ctx);
        return { result: { kind: "tool_result", id: call.id, output, isError: false }, summary };
    }
    catch (error) {
        if (ctx.signal?.aborted === true)
            throw error;
        return {
            result: {
                kind: "tool_result",
                id: call.id,
                output: `${tool.name} failed: ${error.message}`,
                isError: true,
            },
            summary: "failed",
        };
    }
}
