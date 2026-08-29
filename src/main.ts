// Process entry point. The testable bootstrap lives in start.ts.

import { start } from "./start.ts";
import { terminalText } from "./ui/terminal-text.ts";

start().catch((error: unknown) => {
  process.stderr.write(`jecode: ${terminalText((error as Error).message)}\n`);
  process.exitCode = 1;
});
