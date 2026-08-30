// Foreground command and model-turn workflows for the TUI shell.
import { handleCommand } from "../commands.js";
import { runTurn } from "../controller.js";
import { updateSettings } from "../settings.js";
import { saveTranscript } from "../transcript-export.js";
import { recordUsage } from "../usage.js";
import { answerAt, scopeFor } from "./approve.js";
import { controllerOptions, turnFailure } from "./session-view.js";
import { transcribe } from "./turn.js";
const WAITING = "Waiting";
export function appWorkflows(options) {
    const { session, state, allowed, feedback } = options;
    async function command(text) {
        const activity = options.startActivity("command", `Running ${text.split(/\s+/)[0]}`);
        if (activity === undefined)
            return;
        try {
            const outcome = await handleCommand(text, session, {
                emit: options.commandOutput,
                signal: activity.control.signal,
                choose: (picker) => new Promise((resolve) => {
                    state.open = { picker, settle: resolve };
                    options.render();
                }),
                type: (field) => new Promise((resolve) => {
                    state.open = { field, settle: resolve };
                    options.render();
                }),
                status: (said) => {
                    state.status = said ?? activity.label;
                    options.render();
                },
                reset: () => {
                    state.blocks.splice(0);
                    state.past.length = 0;
                    allowed.clear();
                    state.scroll = 0;
                    state.follow = true;
                    state.unseen = 0;
                    state.lastMaxScroll = 0;
                },
                permissions: () => [...allowed].map(([key, label]) => ({ key, label })),
                revokePermission: (key) => {
                    if (key === undefined)
                        allowed.clear();
                    else
                        allowed.delete(key);
                },
                exportTranscript: () => saveTranscript(options.transcriptRoot, state.blocks),
                saveSettings: async (patch) => {
                    await updateSettings(patch);
                },
                refreshSettings: options.refreshSettings,
            });
            if (outcome === "exit")
                state.closeWhenIdle = true;
        }
        catch (error) {
            feedback.show({
                text: activity.control.signal.aborted ? "interrupted" : error.message,
                tone: activity.control.signal.aborted ? "warn" : "error",
                timeoutMs: activity.control.signal.aborted ? 4_200 : 6_000,
            });
        }
        finally {
            options.finishActivity(activity);
        }
    }
    async function turn(text) {
        const activity = options.startActivity("turn", WAITING);
        if (activity === undefined)
            return;
        options.emit({ kind: "user", text });
        session.history.push({ role: "user", content: [{ kind: "text", text }] });
        const events = transcribe({
            emit: options.emit,
            render: options.render,
            palette: session.palette,
            approved: (call) => session.config.autoApprove || allowed.has(scopeFor(call).key),
            remember: (call) => {
                const scope = scopeFor(call);
                allowed.set(scope.key, scope.summary);
            },
            ask: (prompt, settle) => {
                state.open = { picker: prompt, settle: (index) => settle(answerAt(index)) };
                options.render();
            },
            status: (text) => {
                state.status = text;
            },
            usage: (usage) => recordUsage(session.usage, usage),
        });
        try {
            await runTurn(session.history, controllerOptions(session), events, activity.control.signal);
        }
        catch (error) {
            options.emit(turnFailure(session, error, activity.control.signal.aborted));
        }
        finally {
            events.finish();
            options.finishActivity(activity);
        }
    }
    return { command, turn };
}
