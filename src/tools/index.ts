// The tool registry, and the one place a tool actually gets run.

import type { ToolCallBlock, ToolResultBlock, ToolSpec } from "../types.ts";
import type { Tool, ToolContext } from "./types.ts";
import { editFile, listDir, readFile, writeFile } from "./fs.ts";
import { runCommand } from "./shell.ts";
import { findFiles, searchText } from "./search.ts";

export type { Tool, ToolContext, ToolOutput, ToolPreview } from "./types.ts";

/** A finished call: what the model reads, and what the user sees beside it. */
export type ToolRun = { result: ToolResultBlock; summary?: string };

export function builtinTools(): Tool[] {
  return [readFile, listDir, findFiles, searchText, editFile, writeFile, runCommand];
}

export function findTool(tools: Tool[], name: string): Tool | undefined {
  return tools.find((tool) => tool.name === name);
}

/** Strip the executable half — providers only ever see the declaration. */
export function toolSpecs(tools: Tool[]): ToolSpec[] {
  return tools.map((tool) => ({
    name: tool.name,
    description: tool.description,
    input: tool.input,
  }));
}

// A tool failure is a result, not an exception: the model reads the message
// and gets another turn to fix its call. Only an aborted turn propagates.
export async function runTool(
  tool: Tool,
  call: ToolCallBlock,
  ctx: ToolContext,
): Promise<ToolRun> {
  try {
    const { output, summary } = await tool.run(call.input, ctx);
    return { result: { kind: "tool_result", id: call.id, output, isError: false }, summary };
  } catch (error) {
    if (ctx.signal?.aborted === true) throw error;
    return {
      result: {
        kind: "tool_result",
        id: call.id,
        output: `${tool.name} failed: ${(error as Error).message}`,
        isError: true,
      },
      summary: "failed",
    };
  }
}
