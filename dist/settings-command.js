// The persistent settings hub. Every interaction uses the shared dock picker
// and field contracts; this module owns choices and persistence, not drawing.
import { modelsCommand, providersCommand } from "./provider-commands.js";
import { credentialsCommand } from "./credential-commands.js";
import { EFFORTS, readSettings, settingsLabel, updateSettings } from "./settings.js";
import { ollamaConnectionHint, ollamaConnectionSetting, } from "./ollama-settings-command.js";
import { of } from "./tui/editor.js";
import { heading } from "./tui/picker.js";
/** A focused path to the same saved reasoning default exposed by /settings. */
export async function effortCommand(session, host) {
    const value = await effortSetting(session, host);
    if (value === undefined)
        return;
    host.emit({ kind: "notice", text: `effort · ${value}`, tone: "info" });
}
export async function settingsCommand(session, host) {
    const choose = chooser(host);
    if (choose === undefined)
        return;
    let selected = 0;
    while (true) {
        const values = settingsValues(session);
        const items = settingsItems(values);
        const index = await choose(settingsPicker(values, session.palette, selected));
        if (index === undefined)
            return;
        const action = items[index]?.action;
        if (action === undefined || action === "close")
            return;
        selected = index;
        switch (action) {
            case "provider":
                await providerSetting(session, host);
                break;
            case "ollamaConnection":
                await ollamaConnectionSetting(session, host, (patch) => persist(host, patch));
                break;
            case "model":
                await modelSetting(session, host);
                break;
            case "effort":
                await effortSetting(session, host);
                break;
            case "maxTokens":
                await numberSetting(session, host, "maxTokens", "max output tokens");
                break;
            case "maxSteps":
                await numberSetting(session, host, "maxSteps", "max tool steps");
                break;
            case "reducedMotion":
                await motionSetting(session, host);
                break;
            case "credentials":
                await credentialsCommand(session, host);
                break;
        }
    }
}
export function settingsPicker(values, pal, index = 0, store = settingsLabel()) {
    return {
        title: heading("settings", store, pal),
        right: "↑↓ enter · esc close",
        footer: "Changes apply now · flags and environment win at launch",
        options: settingsItems(values).map((item) => item.option),
        index,
    };
}
function settingsItems(values) {
    return [
        { action: "provider", option: { label: "provider", hint: values.provider } },
        ...(values.ollamaConnection === undefined
            ? []
            : [{
                    action: "ollamaConnection",
                    option: { label: "ollama connection", hint: values.ollamaConnection },
                }]),
        { action: "model", option: { label: "model", hint: values.model || "choose a model" } },
        { action: "effort", option: { label: "effort", hint: values.effort } },
        { action: "maxTokens", option: { label: "max output tokens", hint: String(values.maxTokens) } },
        { action: "maxSteps", option: { label: "max tool steps", hint: String(values.maxSteps) } },
        {
            action: "reducedMotion",
            option: { label: "reduced motion", hint: values.reducedMotion ? "on" : "off" },
        },
        { action: "credentials", option: { label: "credentials", hint: "manage API keys" } },
        { action: "close", option: { label: "close" } },
    ];
}
function settingsValues(session) {
    return {
        provider: session.provider.id,
        model: session.model,
        ...(session.provider.id === "ollama" ? { ollamaConnection: ollamaConnectionHint() } : {}),
        effort: session.config.effort,
        maxTokens: session.config.maxTokens,
        maxSteps: session.config.maxSteps,
        reducedMotion: session.config.reducedMotion,
    };
}
async function providerSetting(session, host) {
    const before = {
        provider: session.provider,
        model: session.model,
        providerId: session.config.providerId,
        configModel: session.config.model,
    };
    if (!(await providersCommand(session, host, { announce: false, save: false })))
        return;
    const current = readSettings();
    const models = { ...current.models };
    if (session.model !== "")
        models[session.provider.id] = session.model;
    if (await persist(host, { provider: session.provider.id, models }))
        return;
    session.provider = before.provider;
    session.model = before.model;
    session.config.providerId = before.providerId;
    session.config.model = before.configModel;
}
async function modelSetting(session, host) {
    const before = { model: session.model, configModel: session.config.model };
    if (!(await modelsCommand(session, host, { announce: false, save: false })))
        return;
    const current = readSettings();
    const models = { ...current.models, [session.provider.id]: session.model };
    if (await persist(host, { models }))
        return;
    session.model = before.model;
    session.config.model = before.configModel;
}
async function effortSetting(session, host) {
    const choose = chooser(host);
    if (choose === undefined)
        return;
    const current = session.config.effort;
    const index = await choose({
        title: heading("effort", "saved default", session.palette),
        right: "↑↓ enter · esc back",
        options: EFFORTS.map((value) => ({ label: value })),
        index: Math.max(0, EFFORTS.findIndex((value) => value === current)),
    });
    const value = index === undefined ? undefined : EFFORTS[index];
    if (value === undefined || !(await persist(host, { effort: value })))
        return;
    session.config.effort = value;
    return value;
}
async function motionSetting(session, host) {
    const choose = chooser(host);
    if (choose === undefined)
        return;
    const values = [false, true];
    const index = await choose({
        title: heading("reduced motion", "saved default", session.palette),
        right: "↑↓ enter · esc back",
        options: values.map((value) => ({ label: value ? "on" : "off" })),
        index: session.config.reducedMotion ? 1 : 0,
    });
    const value = index === undefined ? undefined : values[index];
    if (value === undefined || !(await persist(host, { reducedMotion: value })))
        return;
    session.config.reducedMotion = value;
    host.refreshSettings?.();
}
async function numberSetting(session, host, name, label) {
    if (host.type === undefined)
        return;
    const field = {
        title: heading(label, "positive integer", session.palette),
        right: "enter save · esc back",
        editor: of(String(session.config[name])),
        secret: false,
        note: "Applies to the next model turn.",
    };
    const text = await host.type(field);
    if (text === undefined)
        return;
    const value = Number(text);
    if (!Number.isSafeInteger(value) || value <= 0) {
        host.emit({ kind: "notice", text: `${label} must be a positive integer`, tone: "error" });
        return;
    }
    if (!(await persist(host, { [name]: value })))
        return;
    session.config[name] = value;
}
async function persist(host, patch) {
    try {
        await updateSettings(patch);
        return true;
    }
    catch (error) {
        host.emit({ kind: "notice", text: `could not save settings: ${error.message}`, tone: "error" });
        return false;
    }
}
function chooser(host) {
    if (host.choose === undefined) {
        host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
    }
    return host.choose;
}
