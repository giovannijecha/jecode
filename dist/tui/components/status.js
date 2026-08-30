const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
export function spinner(tick) {
    return SPINNER[tick % SPINNER.length];
}
/** Styled content for the footer's replaceable right-hand status channel. */
export function renderStatus(info, pal) {
    const urgent = info.feedback?.tone === "warn" || info.feedback?.tone === "error"
        ? info.feedback
        : undefined;
    if (urgent !== undefined)
        return withUnseen(feedbackSegments(urgent, pal), info.unseen, pal);
    if (info.status !== undefined) {
        return withUnseen([
            { text: `${info.reducedMotion ? "•" : spinner(info.tick)} `, fg: pal.accent },
            { text: `${info.status} (esc to interrupt)`, fg: pal.ink.muted },
        ], info.unseen, pal);
    }
    if (info.feedback !== undefined) {
        return withUnseen(feedbackSegments(info.feedback, pal), info.unseen, pal);
    }
    if (info.unseen > 0)
        return unseenSegments(info.unseen, pal);
    return info.readiness === undefined ? [] : feedbackSegments(info.readiness, pal);
}
function feedbackSegments(feedback, pal) {
    const mark = feedback.tone === "error" ? "×" : feedback.tone === "warn" ? "!" : "·";
    const markColor = {
        info: pal.accent,
        warn: pal.ink.attention,
        error: pal.ink.removed,
    };
    const textColor = feedback.tone === "error" ? pal.ink.removed : pal.ink.muted;
    return [
        { text: `${mark} `, fg: markColor[feedback.tone], bold: true },
        { text: feedback.text, fg: textColor },
    ];
}
function withUnseen(status, unseen, pal) {
    return unseen === 0
        ? status
        : [...status, { text: " · ", fg: pal.ink.muted }, ...unseenSegments(unseen, pal)];
}
function unseenSegments(unseen, pal) {
    return [{ text: `${unseen} new ↓`, fg: pal.accent, bold: true }];
}
