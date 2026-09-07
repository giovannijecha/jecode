// Run existing probes sequentially in fresh processes, retaining failures as evidence.

import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { capture, probeEnvironment } from "./capture.ts";
import { collection, sample, sourceHash } from "./collection.ts";
import { probes } from "./probes.ts";

const [rootArg, outputArg, countArg = "3", ...extra] = process.argv.slice(2);
if (rootArg === undefined || outputArg === undefined || extra.length !== 0 || !/^[1-5]$/.test(countArg)) {
  throw new Error("usage: collect.ts <checkout> <output.json> [repetitions: 1-5, default 3]");
}
const root = resolve(rootArg);
const output = resolve(outputArg);
const report = collection(root, Number(countArg));
// Refuse to replace an existing report or an accidentally selected source file.
await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx" });
const temporary = await mkdtemp(join(tmpdir(), "jecode-benchmark-collection-"));
const abort = new AbortController();
const cancel = (): void => abort.abort();
process.once("SIGINT", cancel);
process.once("SIGTERM", cancel);
try {
  for (const probe of probes) {
    if (abort.signal.aborted) break;
    const entry = { name: probe.name, sourceHash: await sourceHash(root, probe), samples: [] as ReturnType<typeof sample>[] };
    report.probes.push(entry);
    for (let i = 0; i < report.repetitions; i++) {
      if (abort.signal.aborted) break;
      process.stderr.write(`${probe.name}: ${i + 1}/${report.repetitions}\n`);
      const args = [...(probe.name === "tui" ? ["--expose-gc"] : []), `dev/benchmarks/${probe.name}.ts`];
      entry.samples.push(sample(await capture(args, root, probeEnvironment(join(temporary, "home")),
        180_000, 1_048_576, abort.signal), probe));
      await writeFile(output, `${JSON.stringify(report, null, 2)}\n`);
    }
  }
  if (abort.signal.aborted || report.probes.some((probe) => probe.samples.some((sample) => sample.failure !== null))) {
    process.exitCode = 1;
  }
  const after = collection(root, report.repetitions);
  report.dirty ||= after.dirty || after.commit !== report.commit;
  await writeFile(output, `${JSON.stringify(report, null, 2)}\n`);
} finally {
  process.removeListener("SIGINT", cancel);
  process.removeListener("SIGTERM", cancel);
  await rm(temporary, { recursive: true, force: true });
}
