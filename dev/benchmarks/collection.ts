// Development-only collection format. Raw probe reports remain available for review.

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { cpus, release, totalmem } from "node:os";
import { join } from "node:path";
import { isRecord, probes, workload } from "./probes.ts";
import type { Probe } from "./probes.ts";
import type { Capture } from "./capture.ts";

export interface Sample {
  exitCode: number | null;
  failure: string | null;
  stderr: string;
  results: Record<string, unknown> | null;
}

export interface Collection {
  schema: 1;
  commit: string;
  dirty: boolean;
  capturedAt: string;
  environment: Record<string, string | number>;
  repetitions: number;
  probes: Array<{ name: string; sourceHash: string; samples: Sample[] }>;
}

export function collection(root: string, repetitions: number): Collection {
  const git = (args: string[]): string => execFileSync("git", args, {
    cwd: root, encoding: "utf8", windowsHide: true, maxBuffer: 1_048_576,
  }).trim();
  const cpu = cpus();
  return { schema: 1, commit: git(["rev-parse", "HEAD"]),
    dirty: git(["status", "--porcelain", "--untracked-files=normal"]) !== "",
    capturedAt: new Date().toISOString(), repetitions,
    environment: { node: process.version, platform: process.platform, arch: process.arch,
      osRelease: release(), cpu: [...new Set(cpu.map((entry) => entry.model))].sort().join("; "),
      cpuCount: cpu.length, totalMemoryBytes: totalmem(), color: "off",
      runnerImage: process.env["ImageOS"] ?? "", runnerImageVersion: process.env["ImageVersion"] ?? "" },
    probes: [] };
}

export async function sourceHash(root: string, probe: Probe): Promise<string> {
  const hash = createHash("sha256");
  for (const file of [...probe.files, "report.ts"].sort()) {
    hash.update(file).update("\0").update((await readFile(join(root, "dev/benchmarks", file), "utf8"))
      .replaceAll("\r\n", "\n")).update("\0");
  }
  return hash.digest("hex");
}

export function sample(captured: Capture, probe: Probe): Sample {
  const result: Sample = { exitCode: captured.exitCode, failure: captured.failure,
    stderr: captured.stderr, results: null };
  if (result.failure !== null) return result;
  try {
    const report: unknown = JSON.parse(captured.stdout);
    if (!isRecord(report) || report["benchmark"] !== probe.benchmark || !isRecord(report["results"]) ||
      !isRecord(report["environment"]) || report["environment"]["node"] !== process.version ||
      report["environment"]["platform"] !== process.platform || report["environment"]["arch"] !== process.arch) {
      throw new Error("unexpected report");
    }
    workload(report["results"], probe);
    result.results = report["results"];
    if (captured.signal !== null) result.failure = `signal: ${captured.signal}`;
    else if (result.results["passed"] === false && (captured.exitCode === 0 || captured.exitCode === 1)) {
      result.failure = "probe reported failure";
    } else if (captured.exitCode !== 0) result.failure = "unexpected probe exit";
  } catch { result.failure = "missing or invalid probe report"; }
  return result;
}

export function collectionComplete(collection: Collection): boolean {
  return collection.probes.every((probe) => probe.samples.every((sample) => sample.results !== null &&
    (sample.failure === null && sample.exitCode === 0 ||
      sample.failure === "probe reported failure" && sample.results["passed"] === false &&
      (sample.exitCode === 0 || sample.exitCode === 1))));
}

export function validateCollection(value: unknown): Collection {
  if (!isRecord(value) || value["schema"] !== 1 || typeof value["commit"] !== "string" ||
    !/^[a-f0-9]{40}$/.test(value["commit"]) || typeof value["dirty"] !== "boolean" ||
    typeof value["capturedAt"] !== "string" || !Number.isFinite(Date.parse(value["capturedAt"])) ||
    !isRecord(value["environment"]) || !Number.isInteger(value["repetitions"]) ||
    (value["repetitions"] as number) < 1 || (value["repetitions"] as number) > 5 ||
    !Array.isArray(value["probes"]) || value["probes"].length !== probes.length) {
    throw new Error("invalid benchmark collection");
  }
  const environment = value["environment"];
  const fields = ["node", "platform", "arch", "osRelease", "cpu", "color", "runnerImage", "runnerImageVersion"];
  if (fields.some((field) => typeof environment[field] !== "string") ||
    ["cpuCount", "totalMemoryBytes"].some((field) => typeof environment[field] !== "number" ||
      !Number.isFinite(environment[field]) || environment[field] <= 0)) {
    throw new Error("invalid benchmark environment");
  }
  for (const [index, probe] of probes.entries()) {
    const entry: unknown = value["probes"][index];
    if (!isRecord(entry) || entry["name"] !== probe.name || typeof entry["sourceHash"] !== "string" ||
      !/^[a-f0-9]{64}$/.test(entry["sourceHash"]) || !Array.isArray(entry["samples"]) ||
      entry["samples"].length !== value["repetitions"]) throw new Error("incomplete benchmark collection");
    for (const item of entry["samples"]) {
      if (!isRecord(item) || !(item["exitCode"] === null || Number.isInteger(item["exitCode"])) ||
        !(item["failure"] === null || typeof item["failure"] === "string") || typeof item["stderr"] !== "string" ||
        !(item["results"] === null || isRecord(item["results"]))) throw new Error("invalid benchmark sample");
    }
  }
  return value as unknown as Collection;
}
