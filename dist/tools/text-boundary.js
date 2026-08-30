// Shared limits for tools that need to hold an entire text file in memory.
import { constants } from "node:fs";
import { lstat, open } from "node:fs/promises";
export const MAX_EDITABLE_BYTES = 4_000_000;
export const MAX_EDITABLE_CHARS = 1_000_000;
export const MAX_EDITABLE_LINES = 20_000;
const READ_CHUNK_BYTES = 64 * 1024;
/** Read a regular UTF-8 file without allowing an unbounded allocation. */
export async function readEditableText(file, options = {}) {
    const label = options.label ?? "file";
    let details;
    try {
        details = await lstat(file);
    }
    catch (error) {
        if (options.missingAsEmpty === true && isMissing(error))
            return "";
        throw error;
    }
    if (!details.isFile())
        throw new Error(`${label} must be a regular file`);
    let handle;
    try {
        const flags = process.platform === "win32"
            ? "r"
            : constants.O_RDONLY | (constants.O_NONBLOCK ?? 0);
        handle = await open(file, flags);
    }
    catch (error) {
        if (options.missingAsEmpty === true && isMissing(error))
            return "";
        throw error;
    }
    try {
        const stat = await handle.stat();
        if (!stat.isFile())
            throw new Error(`${label} must be a regular file`);
        if (stat.size > MAX_EDITABLE_BYTES) {
            throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
        }
        const chunks = [];
        let total = 0;
        while (total <= MAX_EDITABLE_BYTES) {
            const room = MAX_EDITABLE_BYTES + 1 - total;
            const buffer = Buffer.allocUnsafe(Math.min(READ_CHUNK_BYTES, room));
            const { bytesRead } = await handle.read(buffer, 0, buffer.length, total);
            if (bytesRead === 0)
                break;
            chunks.push(buffer.subarray(0, bytesRead));
            total += bytesRead;
        }
        if (total > MAX_EDITABLE_BYTES) {
            throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
        }
        const text = Buffer.concat(chunks, total).toString("utf8");
        assertEditableText(text, label);
        return text;
    }
    finally {
        await handle.close();
    }
}
/** Reject whole-file writes that exceed Jecode's mutation budget. */
export function assertEditableText(text, label = "content") {
    if (Buffer.byteLength(text, "utf8") > MAX_EDITABLE_BYTES) {
        throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
    }
    if (text.length > MAX_EDITABLE_CHARS) {
        throw limitError(label, `${MAX_EDITABLE_CHARS} characters`);
    }
    if (lineCount(text) > MAX_EDITABLE_LINES) {
        throw limitError(label, `${MAX_EDITABLE_LINES} lines`);
    }
}
/** Check a replacement's size before constructing the resulting string. */
export function assertReplacementFits(before, oldText, newText, replacements) {
    const projectedChars = before.length + replacements * (newText.length - oldText.length);
    if (projectedChars > MAX_EDITABLE_CHARS) {
        throw limitError("edited content", `${MAX_EDITABLE_CHARS} characters`);
    }
    const projectedLines = lineCount(before) +
        replacements * (newlineCount(newText) - newlineCount(oldText));
    if (projectedLines > MAX_EDITABLE_LINES) {
        throw limitError("edited content", `${MAX_EDITABLE_LINES} lines`);
    }
}
function lineCount(text) {
    return newlineCount(text) + 1;
}
function newlineCount(text) {
    let lines = 0;
    for (let index = text.indexOf("\n"); index !== -1; index = text.indexOf("\n", index + 1)) {
        lines += 1;
    }
    return lines;
}
function limitError(label, limit) {
    return new Error(`${label} exceeds the whole-file mutation limit of ${limit}`);
}
function isMissing(error) {
    return error.code === "ENOENT";
}
