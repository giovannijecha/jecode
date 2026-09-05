// Wire foreground workflows to the shell and their shared compaction lifetime.

import { automaticCompactionGate } from "../context/automatic.ts";
import type { AppActions } from "./app-input.ts";
import { commandWorkflow } from "./command-workflow.ts";
import { turnWorkflow } from "./turn-workflow.ts";
import type { WorkflowOptions } from "./workflow-types.ts";

export function appWorkflows(options: WorkflowOptions): AppActions {
  const automaticCompaction = automaticCompactionGate();
  return {
    command: commandWorkflow(options, () => automaticCompaction.reset()),
    ...turnWorkflow(options, automaticCompaction),
  };
}
