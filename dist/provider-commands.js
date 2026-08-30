// Provider, model, and setup command flows.
import { heading } from "./tui/picker.js";
import { PROVIDERS } from "./providers/index.js";
import { readSettings } from "./settings.js";
import { askForKey } from "./credential-commands.js";
/**
 * The provider menu.
 *
 * Every provider is offered, including the ones that cannot run: the reason
 * one is unusable — the variable it wants, by name — is worth more on screen
 * than the row would be worth hidden. A blocked choice opens the same masked
 * credential flow used by setup and settings, and cancellation leaves the old choice.
 */
export async function providersCommand(session, host, behavior = {}) {
    const choose = chooser(host);
    if (choose === undefined)
        return false;
    const options = PROVIDERS.map((provider) => ({
        label: provider.id,
        hint: provider.blocked() ?? "ready",
    }));
    const at = PROVIDERS.findIndex((provider) => provider.id === session.provider.id);
    const index = await choose({
        title: heading("provider", "where the next turn runs", session.palette),
        right: "↑↓ enter",
        options,
        index: Math.max(0, at),
    });
    if (index === undefined)
        return false;
    const chosen = PROVIDERS[index];
    if (chosen === undefined)
        return false;
    // Picking the provider already in use is not a no-op when it cannot run:
    // it is how the user asks to fix the reason it cannot.
    if (chosen.id === session.provider.id) {
        const blocked = chosen.blocked();
        if (blocked !== undefined) {
            if (!isCredentialBlocker(chosen, blocked)) {
                host.emit({ kind: "notice", text: blocked, tone: "error" });
                return false;
            }
            await askForKey(chosen.keyVar, host, session.palette);
            if (chosen.blocked() !== undefined)
                return false;
        }
        return true;
    }
    // A provider that cannot run is worth one offer to fix it, here, rather
    // than a note telling the user to leave and export something.
    const blocked = chosen.blocked();
    if (blocked !== undefined) {
        if (!isCredentialBlocker(chosen, blocked)) {
            host.emit({ kind: "notice", text: blocked, tone: "error" });
            return false;
        }
        await askForKey(chosen.keyVar, host, session.palette);
        const still = chosen.blocked();
        if (still !== undefined) {
            host.emit({
                kind: "notice",
                text: `${providerName(chosen.id)} still needs an API key · provider unchanged`,
                tone: "warn",
            });
            return false;
        }
    }
    const before = {
        provider: session.provider,
        model: session.model,
        providerId: session.config.providerId,
        configModel: session.config.model,
    };
    session.provider = chosen;
    // The model belonged to the old provider. Carrying it across would send
    // `claude-sonnet-5` to OpenAI and call the 404 a bug in the provider.
    session.model = readSettings().models?.[chosen.id] ?? chosen.defaultModel;
    session.config.providerId = chosen.id;
    session.config.model = session.model;
    if (behavior.save !== false) {
        const saved = readSettings();
        const models = { ...saved.models };
        if (session.model !== "")
            models[chosen.id] = session.model;
        if (!(await saveDefaults(host, { provider: chosen.id, models }))) {
            session.provider = before.provider;
            session.model = before.model;
            session.config.providerId = before.providerId;
            session.config.model = before.configModel;
            return false;
        }
    }
    if (behavior.announce !== false) {
        host.emit({
            kind: "notice",
            text: session.model === "" ? `${chosen.id} — pick a model with /models` : `${chosen.id} · ${session.model}`,
            tone: "info",
        });
    }
    return true;
}
/** The model menu, built from what the provider says it has right now. */
export async function modelsCommand(session, host, behavior = {}) {
    // Before the network, not after: asking a provider for a list nothing can
    // be picked from is a request spent on a menu that will never open.
    const choose = chooser(host);
    if (choose === undefined)
        return false;
    const provider = session.provider;
    // Offer the key rather than only naming what is missing: the user came here
    // to pick a model, and "go and export a variable" is not an answer.
    const blocked = provider.blocked();
    if (blocked !== undefined) {
        if (!isCredentialBlocker(provider, blocked)) {
            host.emit({ kind: "notice", text: blocked, tone: "error" });
            return false;
        }
        await askForKey(provider.keyVar, host, session.palette);
        const still = provider.blocked();
        if (still !== undefined) {
            host.emit({
                kind: "notice",
                text: `${providerName(provider.id)} still needs an API key`,
                tone: "warn",
            });
            return false;
        }
    }
    host.status?.(`Asking ${provider.id}`);
    let ids;
    try {
        ids = await provider.models(host.signal, (status) => host.status?.(status));
    }
    catch (error) {
        host.emit({ kind: "notice", text: `${provider.id}: ${error.message}`, tone: "error" });
        return false;
    }
    finally {
        host.status?.(undefined);
    }
    if (ids.length === 0) {
        host.emit({ kind: "notice", text: `${provider.id} offers no models`, tone: "warn" });
        return false;
    }
    const index = await choose({
        title: heading("model", provider.id, session.palette),
        right: "↑↓ enter",
        options: ids.map((id) => ({ label: id })),
        searchable: true,
        query: "",
        index: Math.max(0, ids.indexOf(session.model)),
    });
    if (index === undefined)
        return false;
    const chosen = ids[index];
    if (chosen === undefined)
        return false;
    const before = { model: session.model, configModel: session.config.model };
    session.model = chosen;
    session.config.model = chosen;
    if (behavior.save !== false) {
        const saved = readSettings();
        const models = { ...saved.models, [provider.id]: chosen };
        if (!(await saveDefaults(host, { models }))) {
            session.model = before.model;
            session.config.model = before.configModel;
            return false;
        }
    }
    if (behavior.announce !== false) {
        host.emit({ kind: "notice", text: `${provider.id} · ${chosen}`, tone: "info" });
    }
    return true;
}
/** Make the provider selected by flags/environment usable without leaving the TUI. */
export async function setupCommand(session, host) {
    const blocked = session.provider.blocked();
    if (blocked !== undefined) {
        if (!isCredentialBlocker(session.provider, blocked)) {
            host.emit({ kind: "notice", text: blocked, tone: "error" });
            return;
        }
        const accepted = await askForKey(session.provider.keyVar, host, session.palette);
        if (!accepted || session.provider.blocked() !== undefined) {
            host.emit({
                kind: "notice",
                text: `${providerName(session.provider.id)} still needs an API key · /setup`,
                tone: "warn",
            });
            return;
        }
    }
    if (session.model === "") {
        await modelsCommand(session, host);
        return;
    }
    host.emit({
        kind: "notice",
        text: `${session.provider.id} · ${session.model} · ${session.provider.location?.() ?? "cloud"} · ready`,
        tone: "info",
    });
}
/** The way to put a menu up, or nothing — and the reason, already said. */
function chooser(host) {
    if (host.choose === undefined) {
        host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
    }
    return host.choose;
}
function providerName(id) {
    return id === "" ? "Provider" : `${id[0]?.toUpperCase() ?? ""}${id.slice(1)}`;
}
function isCredentialBlocker(provider, blocked) {
    return blocked.startsWith(`${provider.keyVar} `);
}
async function saveDefaults(host, patch) {
    if (host.saveSettings === undefined)
        return true;
    try {
        await host.saveSettings(patch);
        return true;
    }
    catch (error) {
        host.emit({
            kind: "notice",
            text: `could not save settings · ${error.message}`,
            tone: "error",
        });
        return false;
    }
}
