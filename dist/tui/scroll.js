// Scroll offsets are measured from the bottom. Preserve what is being read as
// the transcript grows, while offset zero keeps following new output.
export function preserveOffset(offset, following, previousMax, nextMax) {
    if (following)
        return 0;
    return Math.min(nextMax, Math.max(0, offset + Math.max(0, nextMax - previousMax)));
}
