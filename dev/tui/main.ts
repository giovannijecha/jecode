// Development entry point; this file is never included in the release runtime.

import { withProcessShutdown, isProcessSignalError } from "../../src/process-shutdown.ts";
import { configureColor } from "../../src/ui/render.ts";
import { terminalText } from "../../src/ui/terminal-text.ts";
import { STEEL } from "../../src/ui/theme.ts";
import { HELP, parseOptions } from "./options.ts";
import { SCENARIOS } from "./registry.ts";
import { TICK_MS } from "./model.ts";
import { composeLab } from "./view.ts";
import { runLab } from "./host.ts";

withProcessShutdown(async (signal) => {
  const options = parseOptions(process.argv.slice(2));
  if (options.mode === "help") { process.stdout.write(HELP); return; }
  if (options.mode === "list") {
    process.stdout.write(SCENARIOS.map((scene) => `${scene.id}\t${scene.group}\t${scene.title}`).join("\n") + "\n");
    return;
  }
  if (options.mode === "render") {
    configureColor(options.color === "auto");
    const frame = composeLab({
      scene: options.scene, palette: STEEL, expanded: true, selected: 0,
      tick: options.time / TICK_MS, reducedMotion: options.reducedMotion,
    }, options.size ?? { rows: 30, cols: 100 });
    process.stdout.write(frame.rows.join("\n") + "\n");
    return;
  }
  await runLab(options, { signal });
}).catch((error: unknown) => {
  if (isProcessSignalError(error)) return;
  process.stderr.write(`jecode tui lab: ${terminalText((error as Error).message)}\n`);
  process.exitCode = 1;
});
