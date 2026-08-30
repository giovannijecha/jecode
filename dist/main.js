// Process entry point. The testable bootstrap lives in start.ts.
import { start } from "./start.js";
import { terminalText } from "./ui/terminal-text.js";
start().catch((error) => {
    process.stderr.write(`jecode: ${terminalText(error.message)}\n`);
    process.exitCode = 1;
});
