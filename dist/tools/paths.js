import * as path from "node:path";
import { lstat, realpath } from "node:fs/promises";
// Every filesystem tool resolves through here. The agent works inside one
// root and cannot be talked into reaching outside it — including via "..",
// an absolute path, or (on Windows) a different drive letter.
export function resolveInRoot(root, candidate) {
    const absoluteRoot = path.resolve(root);
    const absolute = path.resolve(absoluteRoot, candidate);
    if (!inside(absoluteRoot, absolute)) {
        throw new Error(`path escapes the workspace root: ${candidate}`);
    }
    return absolute;
}
/** Resolve an existing path after following symlinks and Windows junctions. */
export async function resolveExistingInRoot(root, candidate) {
    const lexical = resolveInRoot(root, candidate);
    const canonicalRoot = await realpath(root);
    const canonical = await realpath(lexical);
    if (!inside(canonicalRoot, canonical))
        throw new Error(`path escapes the workspace root: ${candidate}`);
    return canonical;
}
/** Resolve a path that may not exist, canonicalizing its nearest existing parent. */
export async function resolveWritableInRoot(root, candidate) {
    const lexical = resolveInRoot(root, candidate);
    const canonicalRoot = await realpath(root);
    try {
        const canonical = await realpath(lexical);
        if (!inside(canonicalRoot, canonical))
            throw new Error(`path escapes the workspace root: ${candidate}`);
        return canonical;
    }
    catch (error) {
        if (!missing(error))
            throw error;
    }
    const rest = [path.basename(lexical)];
    let parent = path.dirname(lexical);
    while (true) {
        try {
            const canonicalParent = await realpath(parent);
            const target = path.join(canonicalParent, ...rest);
            if (!inside(canonicalRoot, target))
                throw new Error(`path escapes the workspace root: ${candidate}`);
            return target;
        }
        catch (error) {
            if (!missing(error))
                throw error;
            const next = path.dirname(parent);
            if (next === parent)
                throw error;
            rest.unshift(path.basename(parent));
            parent = next;
        }
    }
}
/**
 * Resolve a write target without permitting a symlink or junction component.
 *
 * Reads may follow an in-workspace alias. Writes stay on the direct path so a
 * later boundary check can detect a component replaced during the operation.
 */
export async function resolveDirectWritableInRoot(root, candidate, mustExist = false) {
    const { canonicalRoot, target } = await directPath(root, candidate);
    await assertDirectWritableInRoot(canonicalRoot, target, mustExist);
    return target;
}
/** Revalidate a previously resolved direct write target. */
export async function assertDirectWritableInRoot(root, target, mustExist = false) {
    const { canonicalRoot, target: direct } = await directPath(root, target);
    const relative = path.relative(canonicalRoot, direct);
    let current = canonicalRoot;
    for (const part of relative.split(path.sep).filter((value) => value !== "")) {
        current = path.join(current, part);
        try {
            const details = await lstat(current);
            if (details.isSymbolicLink())
                throw writeLinkError();
        }
        catch (error) {
            if (!missing(error))
                throw error;
            if (mustExist)
                throw error;
            break;
        }
    }
    const resolved = mustExist
        ? await realpath(direct)
        : await resolveWritableInRoot(canonicalRoot, direct);
    if (!samePath(direct, resolved))
        throw writeLinkError();
}
async function directPath(root, candidate) {
    const lexicalRoot = path.resolve(root);
    const lexicalTarget = resolveInRoot(lexicalRoot, candidate);
    const relative = path.relative(lexicalRoot, lexicalTarget);
    const canonicalRoot = await realpath(lexicalRoot);
    const target = path.resolve(canonicalRoot, relative);
    if (!inside(canonicalRoot, target)) {
        throw new Error(`path escapes the workspace root: ${candidate}`);
    }
    return { canonicalRoot, target };
}
export function displayPath(root, absolute) {
    const relative = path.relative(root, absolute);
    return relative === "" ? "." : relative.split(path.sep).join("/");
}
function inside(root, target) {
    const relative = path.relative(root, target);
    return relative === "" || (!path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`));
}
function samePath(left, right) {
    return path.relative(left, right) === "";
}
function writeLinkError() {
    return new Error("write path contains a symbolic link or junction — use a direct workspace path");
}
function missing(error) {
    return error.code === "ENOENT";
}
