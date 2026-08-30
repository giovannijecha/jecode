// One foreground operation owns cancellation and elapsed time.
export function begin(kind, label, now = Date.now()) {
    return { kind, label, control: new AbortController(), startedAt: now };
}
export function elapsed(activity, now = Date.now()) {
    const seconds = Math.max(0, Math.floor((now - activity.startedAt) / 1_000));
    if (seconds < 60)
        return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
}
