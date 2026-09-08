// Refill shared-read slots as they settle. Indexes preserve wire order, while
// a processing failure stops launching new work and waits for active reads.
export async function settlePool<T>(
  jobs: readonly (() => Promise<T>)[],
  limit: number,
): Promise<PromiseSettledResult<T>[]> {
  const results: PromiseSettledResult<T>[] = new Array(jobs.length);
  let next = 0;
  let failure: { reason: unknown } | undefined;
  const worker = async (): Promise<void> => {
    while (next < jobs.length) {
      const index = next++;
      if (failure !== undefined) {
        results[index] = { status: "rejected", reason: failure.reason };
        continue;
      }
      try { results[index] = { status: "fulfilled", value: await jobs[index]!() }; }
      catch (reason) {
        failure ??= { reason };
        results[index] = { status: "rejected", reason };
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, jobs.length) }, worker));
  return results;
}
