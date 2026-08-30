// Bounded, read-only workspace discovery without borrowing a shell.
import * as fs from "node:fs/promises";
import * as path from "node:path";
import { optionalBool, optionalInt, optionalString, requireString } from "./args.js";
import { displayPath, resolveExistingInRoot } from "./paths.js";
const DEFAULT_RESULTS = 100;
const MAX_RESULTS = 500;
const MAX_VISITED = 20_000;
const MAX_FILE_BYTES = 1_000_000;
const MAX_MATCH_LINE = 500;
const SKIP = new Set([".git", ".hg", ".svn", "node_modules"]);
export const findFiles = {
    name: "find_files",
    description: "Find files inside the workspace by glob (for example **/*.ts). Skips dependency and VCS " +
        "directories, never follows symlinks, and returns a bounded list.",
    dangerous: false,
    input: {
        type: "object",
        properties: {
            pattern: { type: "string", description: "Glob matched against workspace-relative paths." },
            path: { type: "string", description: "Directory to search, relative to the workspace root." },
            max_results: { type: "integer", description: "Maximum paths returned. Defaults to 100, caps at 500." },
        },
        required: ["pattern"],
    },
    async run(args, ctx) {
        const scoped = await canonicalContext(ctx);
        const start = await startAt(args, scoped);
        const match = glob(requireString(args, "pattern"));
        const limit = resultLimit(args);
        const found = [];
        const walked = await walk(start, scoped, async (file) => {
            const relative = displayPath(scoped.root, file);
            if (match(relative))
                found.push(relative);
            return found.length >= limit;
        });
        found.sort((a, b) => a.localeCompare(b));
        return {
            output: found.length === 0 ? "[no matching files]" : found.join("\n"),
            summary: summary(found.length, limit, walked.capped, "file", "files"),
        };
    },
};
export const searchText = {
    name: "search_text",
    description: "Search UTF-8 text files inside the workspace for a literal string. Skips dependencies, VCS " +
        "directories, symlinks, binary files, and files over 1 MB; results are bounded.",
    dangerous: false,
    input: {
        type: "object",
        properties: {
            query: { type: "string", description: "Literal text to find." },
            path: { type: "string", description: "Directory to search, relative to the workspace root." },
            pattern: { type: "string", description: "Optional file glob, for example **/*.ts." },
            case_sensitive: { type: "boolean", description: "Defaults to false." },
            max_results: { type: "integer", description: "Maximum matching lines. Defaults to 100, caps at 500." },
        },
        required: ["query"],
    },
    async run(args, ctx) {
        const scoped = await canonicalContext(ctx);
        const start = await startAt(args, scoped);
        const query = requireString(args, "query");
        const sensitive = optionalBool(args, "case_sensitive") ?? false;
        const needle = sensitive ? query : query.toLocaleLowerCase();
        const pattern = optionalString(args, "pattern");
        const match = pattern === undefined || pattern === "" ? () => true : glob(pattern);
        const limit = resultLimit(args);
        const found = [];
        let skipped = 0;
        const walked = await walk(start, scoped, async (lexical) => {
            const relative = displayPath(scoped.root, lexical);
            if (!match(relative))
                return false;
            const file = await resolveExistingInRoot(scoped.root, lexical);
            const info = await fs.stat(file);
            if (info.size > MAX_FILE_BYTES) {
                skipped++;
                return false;
            }
            let text;
            try {
                const data = await fs.readFile(file);
                if (data.includes(0)) {
                    skipped++;
                    return false;
                }
                text = data.toString("utf8");
            }
            catch (error) {
                if (skippable(error)) {
                    skipped++;
                    return false;
                }
                throw error;
            }
            for (const [index, line] of text.replace(/\r\n?/g, "\n").split("\n").entries()) {
                checkAbort(ctx.signal);
                const haystack = sensitive ? line : line.toLocaleLowerCase();
                if (!haystack.includes(needle))
                    continue;
                found.push(`${relative}:${index + 1}:${clip(line)}`);
                if (found.length >= limit)
                    return true;
            }
            return false;
        });
        const extra = skipped === 0 ? "" : ` · skipped ${skipped} binary/large/unreadable`;
        return {
            output: found.length === 0 ? "[no matches]" : found.join("\n"),
            summary: `${summary(found.length, limit, walked.capped, "match", "matches")}${extra}`,
        };
    },
};
async function walk(start, ctx, visit) {
    const pending = [start];
    let seen = 0;
    while (pending.length > 0) {
        checkAbort(ctx.signal);
        const lexical = pending.pop();
        const directory = await resolveExistingInRoot(ctx.root, lexical);
        let entries;
        try {
            entries = await fs.readdir(directory, { withFileTypes: true, encoding: "utf8" });
        }
        catch (error) {
            if (skippable(error))
                continue;
            throw error;
        }
        entries.sort((a, b) => a.name.localeCompare(b.name));
        for (const entry of entries) {
            checkAbort(ctx.signal);
            if (++seen > MAX_VISITED)
                return { capped: true };
            if (entry.isSymbolicLink())
                continue;
            const target = path.join(directory, entry.name);
            if (entry.isDirectory()) {
                if (!SKIP.has(entry.name))
                    pending.push(target);
            }
            else if (entry.isFile() && (await visit(target))) {
                return { capped: false };
            }
        }
    }
    return { capped: false };
}
async function startAt(args, ctx) {
    const candidate = optionalString(args, "path") ?? ".";
    const target = await resolveExistingInRoot(ctx.root, candidate);
    if (!(await fs.stat(target)).isDirectory())
        throw new Error(`"path" is not a directory: ${candidate}`);
    return target;
}
async function canonicalContext(ctx) {
    return { ...ctx, root: await resolveExistingInRoot(ctx.root, ".") };
}
function resultLimit(args) {
    const requested = optionalInt(args, "max_results") ?? DEFAULT_RESULTS;
    if (requested <= 0)
        throw new Error('"max_results" must be a positive integer');
    return Math.min(requested, MAX_RESULTS);
}
function glob(pattern) {
    const normalized = pattern.replace(/\\/g, "/");
    let source = "";
    for (let index = 0; index < normalized.length; index++) {
        const char = normalized[index];
        if (char === "*" && normalized[index + 1] === "*") {
            if (normalized[index + 2] === "/") {
                source += "(?:.*/)?";
                index += 2;
            }
            else {
                source += ".*";
                index++;
            }
        }
        else if (char === "*")
            source += "[^/]*";
        else if (char === "?")
            source += "[^/]";
        else
            source += char.replace(/[|\\{}()[\]^$+?.]/g, "\\$&");
    }
    const expression = new RegExp(`^${source}$`, "i");
    const basenameOnly = !normalized.includes("/");
    return (relative) => expression.test(basenameOnly ? path.posix.basename(relative) : relative);
}
function summary(count, limit, capped, one, many) {
    const noun = count === 1 ? one : many;
    if (count >= limit)
        return `${count} ${noun} · result limit`;
    if (capped)
        return `${count} ${noun} · scan limit`;
    return `${count} ${noun}`;
}
function clip(line) {
    if (line.length <= MAX_MATCH_LINE)
        return line;
    return `${line.slice(0, MAX_MATCH_LINE - 1)}…`;
}
function checkAbort(signal) {
    if (signal?.aborted !== true)
        return;
    throw signal.reason instanceof Error ? signal.reason : new Error("aborted");
}
function skippable(error) {
    return ["EACCES", "EPERM", "ENOENT"].includes(error.code ?? "");
}
