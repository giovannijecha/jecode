// Run the production TUI with opt-in, content-free context diagnostics.

import { start } from "../../src/start.ts";
import { isProcessSignalError, withProcessShutdown } from "../../src/process-shutdown.ts";
import { terminalText } from "../../src/ui/terminal-text.ts";
import { contextRecorder } from "./recorder.ts";

withProcessShutdown(async (signal) => {
  const recorder = await contextRecorder();
  try {
    await start(process.argv.slice(2), { signal });
  } finally {
    const result = await recorder.close();
    process.stderr.write(`Context diagnostics: ${terminalText(recorder.file)}\n`);
    process.stderr.write(`${result.written} records; ${result.dropped} dropped` +
      (result.writeFailed ? "; recording stopped after an I/O failure\n" : "\n"));
  }
}).catch((error: unknown) => {
  if (isProcessSignalError(error)) return;
  process.stderr.write(`jecode: ${terminalText((error as Error).message)}\n`);
  process.exitCode = 1;
});
