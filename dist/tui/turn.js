// Turning what the controller reports into what the transcript shows.
//
// The controller speaks in stream events and tool results; the screen speaks in
// blocks. This is the whole of the translation, kept out of the shell so that
// neither has to know how the other is built.
import { condense, diff } from "../ui/diff.js";
import { promptFor } from "./approve.js";
/**
 * What the footer can truthfully say.
 *
 * Every one of these is read off an event that has actually arrived, never
 * from a guess about where the turn probably is. A status line that says one
 * thing for the whole turn is decoration; this one is the only window into a
 * process that is otherwise silent for seconds at a time, so it has to be
 * worth believing.
 */
const WAITING = "Waiting";
const THINKING = "Thinking";
const WRITING = "Writing";
const ASKING = "Waiting for you";
/** Unchanged rows kept either side of a change. */
const CONTEXT = 2;
export function transcribe(stage) {
    // The block the stream is currently filling. A change of kind starts a new
    // one, which is what keeps reasoning and answer from running together.
    let open;
    const tools = new Map();
    let step = 1;
    let steps = 1;
    let tool = 1;
    let toolTotal = 1;
    const close = () => {
        const block = open?.block;
        const changed = block?.kind === "reasoning" && block.live === true;
        if (changed && block.kind === "reasoning")
            block.live = false;
        open = undefined;
        return changed ? block : undefined;
    };
    return {
        finish() {
            const changed = close();
            if (changed !== undefined)
                stage.render(changed);
        },
        onStep(current, total) {
            step = current;
            steps = total;
            stage.status(waiting(step, steps));
            stage.render();
        },
        onToolProgress(current, total) {
            tool = current;
            toolTotal = total;
        },
        onUsage(usage) {
            stage.usage?.(usage);
        },
        onStatus(status) {
            stage.status(status);
            stage.render();
        },
        onStream(event) {
            const kind = event.kind === "thinking" ? "reasoning" : "answer";
            stage.status(kind === "reasoning" ? THINKING : WRITING);
            if (open === undefined || open.kind !== kind) {
                const changed = close();
                if (changed !== undefined)
                    stage.render(changed);
                const block = kind === "reasoning"
                    ? { kind, text: "", live: true, expanded: false }
                    : { kind, text: "" };
                open = { kind, block };
                stage.emit(block);
            }
            if (open.block.kind === "answer" || open.block.kind === "reasoning") {
                open.block.text += event.text;
            }
            stage.render(open.block);
        },
        onToolCall(call, look) {
            const changed = close();
            if (changed !== undefined)
                stage.render(changed);
            stage.status(`Running ${call.name}${toolTotal > 1 ? ` · tool ${tool}/${toolTotal}` : ""}${step > 1 ? ` · step ${step}/${steps}` : ""}`);
            const block = {
                kind: "tool",
                name: call.name,
                target: target(call.input),
                right: "pending",
                tone: "pending",
                body: preview(call, look),
            };
            tools.set(call.id, block);
            stage.emit(block);
            stage.render(block);
        },
        onToolResult(call, result, summary) {
            stage.status(waiting(step, steps));
            const block = tools.get(call.id);
            if (block === undefined || block.kind !== "tool")
                return;
            block.tone = result.isError ? "fail" : "ok";
            block.right = summary ?? "";
            // A failure replaces the preview: what the call was going to do stops
            // being the interesting part the moment it did not do it.
            const outcome = result.isError
                ? details(result.output, "out")
                : produced(call, result.output);
            if (outcome !== undefined)
                block.body = outcome;
            stage.render(block);
        },
        approve(call) {
            if (stage.approved(call))
                return Promise.resolve(true);
            const changed = close();
            if (changed !== undefined)
                stage.render(changed);
            stage.status(ASKING);
            return new Promise((resolve) => {
                stage.ask(promptFor(call, target(call.input), stage.palette), (answer) => {
                    if (answer === "always")
                        stage.remember(call);
                    const approved = answer !== "no";
                    const block = tools.get(call.id);
                    if (!approved && block !== undefined && block.kind === "tool") {
                        block.tone = "deny";
                        block.right = "declined";
                        block.body = undefined;
                    }
                    stage.status(approved ? `Running ${call.name}` : waiting(step, steps));
                    stage.render(block);
                    resolve(approved);
                });
            });
        },
    };
}
function waiting(step, total) {
    return step === 1 ? WAITING : `${WAITING} · step ${step}/${total}`;
}
/** The argument worth showing: the thing the call acts on. */
function target(input) {
    const { path, command } = input;
    if (typeof command === "string")
        return command;
    if (typeof path === "string")
        return path;
    const rest = JSON.stringify(input);
    return rest === "{}" ? "" : rest;
}
/**
 * What the call is about to do, drawn before it is allowed to do it.
 *
 * The tool is asked first, because only it knows what is already on disk: a
 * write against an existing file is a replacement, and showing it as a page of
 * additions hides exactly the part worth approving. The fallback diffs what is
 * in the arguments, which is all there is when a tool has nothing to say.
 */
function preview(call, look) {
    if (look !== undefined)
        return changes(look.before, look.after);
    const input = call.input;
    if (call.name === "write_file" && typeof input.content === "string") {
        return changes("", input.content);
    }
    if (call.name === "edit_file" && typeof input.old_text === "string") {
        return changes(input.old_text, typeof input.new_text === "string" ? input.new_text : "");
    }
    return undefined;
}
/** Two texts as the rows of their difference, unchanged runs summed up. */
function changes(before, after) {
    let oldLine = 1;
    let newLine = 1;
    const rows = [];
    for (const changed of condense(diff(before, after), CONTEXT)) {
        if (changed.kind === "gap") {
            rows.push({ kind: "gap", text: `… ${changed.skipped} unchanged` });
            oldLine += changed.skipped;
            newLine += changed.skipped;
            continue;
        }
        if (changed.kind === "keep") {
            rows.push({ kind: "keep", text: tabs(changed.text), oldLine, newLine });
            oldLine++;
            newLine++;
            continue;
        }
        if (changed.kind === "del") {
            rows.push({ kind: "del", text: tabs(changed.text), oldLine });
            oldLine++;
            continue;
        }
        rows.push({ kind: "add", text: tabs(changed.text), newLine });
        newLine++;
    }
    emphasizePairs(rows);
    return rows.length === 0 ? undefined : rows;
}
function emphasizePairs(rows) {
    for (let index = 0; index < rows.length - 1; index++) {
        const removed = rows[index];
        const added = rows[index + 1];
        if (removed?.kind !== "del" || added?.kind !== "add")
            continue;
        if (rows[index - 1]?.kind === "del" || rows[index + 2]?.kind === "add")
            continue;
        let start = 0;
        while (start < removed.text.length && start < added.text.length && removed.text[start] === added.text[start])
            start++;
        let suffix = 0;
        while (suffix < removed.text.length - start &&
            suffix < added.text.length - start &&
            removed.text[removed.text.length - 1 - suffix] === added.text[added.text.length - 1 - suffix])
            suffix++;
        while (suffix > 0 && (!wordBoundary(removed.text, suffix) || !wordBoundary(added.text, suffix)))
            suffix--;
        const removedLength = removed.text.length - start - suffix;
        const addedLength = added.text.length - start - suffix;
        if (removedLength > 0)
            removed.emphasis = { start, length: removedLength };
        if (addedLength > 0)
            added.emphasis = { start, length: addedLength };
    }
}
function wordBoundary(text, suffix) {
    const at = text.length - suffix;
    if (at <= 0 || at >= text.length)
        return true;
    return /\w/.test(text[at - 1]) !== /\w/.test(text[at]);
}
/** What the call left behind, for the calls whose output is worth a look. */
function produced(call, output) {
    if (call.name === "run_command")
        return details(output, "out");
    // A read or a listing is already summed up on the right of its own row, and
    // the model is about to say what was in it. Printing it twice helps nobody.
    return undefined;
}
function details(text, kind) {
    const rows = split(text).map((line) => ({ kind, text: tabs(line) }));
    return rows.length === 0 ? undefined : rows;
}
function split(text) {
    const lines = text.replace(/\r\n?/g, "\n").split("\n");
    while (lines.length > 0 && lines[lines.length - 1] === "")
        lines.pop();
    return lines.map(tabs);
}
// A tab is a width the terminal decides and the row measurement cannot see.
function tabs(line) {
    return line.replace(/\t/g, "  ");
}
