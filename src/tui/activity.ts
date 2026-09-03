// One foreground operation owns cancellation and elapsed time.

export type ActivityKind = "turn" | "command";

export type Activity = {
  kind: ActivityKind;
  label: string;
  control: AbortController;
  startedAt: number;
  phase: {
    label: string;
    startedAt: number;
  };
};

export function begin(kind: ActivityKind, label: string, now = Date.now()): Activity {
  return {
    kind,
    label,
    control: new AbortController(),
    startedAt: now,
    phase: { label, startedAt: now },
  };
}

/** Change the visible phase without restarting its timer on repeated stream chunks. */
export function transition(activity: Activity, label: string, now = Date.now()): void {
  if (activity.phase.label === label) return;
  activity.phase = { label, startedAt: now };
}

export function elapsed(activity: Activity, now = Date.now()): string {
  return since(activity.startedAt, now);
}

export function activityStatus(
  activity: Activity,
  now = Date.now(),
): string {
  return `${activity.phase.label} · ${since(activity.phase.startedAt, now)}`;
}

function since(startedAt: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1_000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
}
