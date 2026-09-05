// Manual integrated TUI probe. Stdout contains JSON, never terminal frames.

import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { reportBenchmark } from "./report.ts";

const before = process.env["JECODE_HOME"];
// Dynamic imports keep real account stores out of module initialization, too.
process.env["JECODE_HOME"] = path.join(tmpdir(), `jecode-tui-bench-${randomUUID()}`);
try {
  const { scenario } = await import("./tui-scenario.ts");
  const { hasColor } = await import("../../src/ui/render.ts");
  const results = [];
  for (const [columns, blocks, rate] of [[120, 0], [120, 2_000], [60, 2_000], [120, 2_000, 32_768]] as const) {
    results.push(await scenario(columns, blocks, rate));
  }
  reportBenchmark("tui-responsiveness", {
    color: hasColor(), reducedMotion: true, forcedGcBetweenPhases: true,
    measurement: "raw input dispatch through scheduled frame and real painter; sink acknowledgement is not display latency",
    results,
  });
} finally {
  if (before === undefined) delete process.env["JECODE_HOME"];
  else process.env["JECODE_HOME"] = before;
}
