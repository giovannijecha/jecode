// One machine-readable envelope for the independent manual performance probes.

export function reportBenchmark(benchmark: string, results: Record<string, unknown>): void {
  process.stdout.write(`${JSON.stringify({
    benchmark,
    environment: {
      node: process.version,
      platform: process.platform,
      arch: process.arch,
    },
    results,
  })}\n`);
}

export function round(value: number): number {
  return Number(value.toFixed(3));
}
