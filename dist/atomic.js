// Crash-safe replacement of one file, using a verified temporary sibling and
// rename.
import { lstat, open, rename, rm, stat } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import * as path from "node:path";
export async function atomicWrite(file, content, options = {}) {
    const temporary = path.join(path.dirname(file), `.${path.basename(file)}.${process.pid}.${randomUUID()}.tmp`);
    let handle;
    let identity;
    try {
        await options.validate?.("before-open");
        const permissions = options.mode ?? (await existingMode(file));
        handle = await open(temporary, "wx", permissions);
        identity = fileIdentity(await handle.stat());
        await options.validate?.("before-write");
        await assertNamedFile(temporary, identity);
        await handle.writeFile(content, "utf8");
        if (permissions !== undefined && process.platform !== "win32") {
            await handle.chmod(permissions);
        }
        await handle.sync();
        identity = fileIdentity(await handle.stat());
        await options.validate?.("before-rename");
        await assertNamedFile(temporary, identity);
        await rename(temporary, file);
        const completed = handle;
        handle = undefined;
        await completed.close().catch(() => undefined);
    }
    catch (error) {
        if (handle !== undefined) {
            await handle.truncate(0).catch(() => undefined);
            await handle.sync().catch(() => undefined);
            await handle.close().catch(() => undefined);
            handle = undefined;
        }
        await removeTemporary(temporary, identity, options.validate);
        throw error;
    }
}
async function assertNamedFile(file, expected) {
    const details = await lstat(file);
    if (details.isSymbolicLink() || !sameFile(expected, fileIdentity(details))) {
        throw new Error("atomic write target changed during replacement");
    }
}
async function removeTemporary(file, expected, validate) {
    if (expected === undefined)
        return;
    try {
        await validate?.("before-cleanup");
        await assertNamedFile(file, expected);
        await rm(file);
    }
    catch {
        // A changed path is no longer ours to remove.
    }
}
function fileIdentity(details) {
    return { dev: details.dev, ino: details.ino, birthtimeMs: details.birthtimeMs };
}
function sameFile(left, right) {
    if (left.dev !== right.dev || left.ino !== right.ino)
        return false;
    return left.ino !== 0 || left.birthtimeMs === right.birthtimeMs;
}
async function existingMode(file) {
    if (process.platform === "win32")
        return undefined;
    try {
        return (await stat(file)).mode & 0o777;
    }
    catch (error) {
        if (error.code === "ENOENT")
            return undefined;
        throw error;
    }
}
