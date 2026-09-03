// One compact duration grammar shared by terminal and exported tool evidence.

export function toolDuration(durationMs: number, live = false): string {
  const bounded = Number.isFinite(durationMs) ? Math.max(0, durationMs) : 0;
  if (!live && bounded < 1_000) return `${Math.round(bounded)}ms`;
  const seconds = bounded / 1_000;
  return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`;
}
