// Credential command flows shared by setup and provider selection.
import { heading } from "./tui/picker.js";
import { EMPTY } from "./tui/editor.js";
import { PROVIDERS } from "./providers/index.js";
import { credentialSource, forgetSaved, hasSaved, hold, keep, storeLabel, } from "./credentials.js";
/** Ask for a key, then ask separately whether it may be written to disk. */
export async function askForKey(name, host, pal) {
    if (host.type === undefined || host.choose === undefined)
        return false;
    const field = {
        title: [
            { text: "paste key  ", fg: pal.ink.attention, bold: true },
            { text: name, fg: pal.ink.bright, bold: true },
        ],
        right: "enter ok · esc skip",
        editor: EMPTY,
        secret: true,
        note: "read from the environment first — an exported value still wins",
    };
    const value = await host.type(field);
    if (value === undefined)
        return false;
    const index = await host.choose({
        title: heading("remember it?", name, pal),
        right: "↑↓ enter",
        options: [
            { label: "just this session", hint: "nothing is written", key: "s" },
            { label: `save it to ${storeLabel()}`, hint: "owner-only file", key: "w" },
            { label: "discard it", key: "d" },
        ],
        index: 0,
    });
    if (index === 0) {
        hold(name, value);
        host.emit({ kind: "notice", text: "credential available for this session", tone: "info" });
        return true;
    }
    if (index === 1) {
        try {
            await keep(name, value);
            host.emit({ kind: "notice", text: `credential saved · ${storeLabel()}`, tone: "info" });
            return true;
        }
        catch (error) {
            host.emit({ kind: "notice", text: `could not save credential · ${error.message}`, tone: "error" });
            return false;
        }
    }
    host.emit({ kind: "notice", text: "credential discarded", tone: "warn" });
    return false;
}
export async function credentialsCommand(session, host) {
    const choose = chooser(host);
    if (choose === undefined)
        return;
    const index = await choose({
        title: heading("credential", "values are never shown", session.palette),
        right: "↑↓ enter · esc close",
        options: PROVIDERS.map((provider) => ({
            label: provider.keyVar,
            hint: credentialSource(provider.keyVar) ?? "missing",
        })),
        index: Math.max(0, PROVIDERS.findIndex((provider) => provider.id === session.provider.id)),
    });
    if (index === undefined)
        return;
    const provider = PROVIDERS[index];
    if (provider === undefined)
        return;
    const name = provider.keyVar;
    const source = credentialSource(name);
    if (source === "environment") {
        host.emit({
            kind: "notice",
            text: `${name} comes from the environment · update it outside jecode and restart`,
            tone: "info",
        });
        if (hasSaved(name))
            await offerForget(name, session, host, "a saved copy is currently shadowed");
        return;
    }
    const actions = [
        { label: source === undefined ? "add credential" : "replace credential", key: "r" },
        ...(hasSaved(name) ? [{ label: "forget saved copy", hint: storeLabel(), key: "f" }] : []),
        { label: "close", key: "c" },
    ];
    const action = await choose({
        title: heading(name, source ?? "missing", session.palette),
        right: "↑↓ enter",
        options: actions,
        index: 0,
    });
    if (action === undefined || action === actions.length - 1)
        return;
    if (actions[action]?.key === "f") {
        await forget(name, host);
        return;
    }
    await askForKey(name, host, session.palette);
}
async function offerForget(name, session, host, hint) {
    if (host.choose === undefined)
        return;
    const index = await host.choose({
        title: heading(name, hint, session.palette),
        right: "↑↓ enter",
        options: [
            { label: "keep saved copy", key: "k" },
            { label: "forget saved copy", hint: storeLabel(), key: "f" },
        ],
        index: 0,
    });
    if (index === 1)
        await forget(name, host);
}
async function forget(name, host) {
    try {
        const removed = await forgetSaved(name);
        host.emit({
            kind: "notice",
            text: removed ? "saved credential removed" : "no saved credential to remove",
            tone: removed ? "info" : "warn",
        });
    }
    catch (error) {
        host.emit({ kind: "notice", text: `could not forget: ${error.message}`, tone: "error" });
    }
}
function chooser(host) {
    if (host.choose === undefined) {
        host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
    }
    return host.choose;
}
