// Crash-safe replacement of one file, using a temporary sibling and rename.
import { open, chmod, rename, rm, stat } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import * as path from "node:path";
export async function atomicWrite(file, content, mode) {
    const permissions = mode ?? (await existingMode(file));
    const temporary = path.join(path.dirname(file), `.${path.basename(file)}.${process.pid}.${randomUUID()}.tmp`);
    let handle;
    try {
        handle = await open(temporary, "wx", permissions);
        await handle.writeFile(content, "utf8");
        await handle.sync();
        await handle.close();
        handle = undefined;
        await rename(temporary, file);
        // `open` applies the process umask, so explicitly restore a requested or
        // pre-existing mode after the rename. Editing a script must not make it
        // stop being executable.
        if (permissions !== undefined && process.platform !== "win32")
            await chmod(file, permissions);
    }
    catch (error) {
        await handle?.close().catch(() => undefined);
        await rm(temporary, { force: true }).catch(() => undefined);
        throw error;
    }
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
