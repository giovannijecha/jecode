// One foreground operation owns cancellation and elapsed time.

export type ActivityKind = "turn" | "command";

export type Activity = {
  kind: ActivityKind;
  label: string;
  control: AbortController;
  startedAt: number;
};

export function begin(kind: ActivityKind, label: string, now = Date.now()): Activity {
  return { kind, label, control: new AbortController(), startedAt: now };
}

export function elapsed(activity: Activity, now = Date.now()): string {
  const seconds = Math.max(0, Math.floor((now - activity.startedAt) / 1_000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
}

export function activityStatus(
  activity: Activity,
  label = activity.label,
  now = Date.now(),
): string {
  return `${label} · ${elapsed(activity, now)}`;
}
