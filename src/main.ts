// Process entry point. The testable bootstrap lives in start.ts.

import { start } from "./start.ts";
import { isProcessSignalError, withProcessShutdown } from "./process-shutdown.ts";
import { terminalText } from "./ui/terminal-text.ts";

withProcessShutdown((signal) => start(process.argv.slice(2), { signal })).catch((error: unknown) => {
  if (isProcessSignalError(error)) return;
  process.stderr.write(`jecode: ${terminalText((error as Error).message)}\n`);
  process.exitCode = 1;
});
