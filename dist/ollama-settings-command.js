// Ollama connection choices inside the persistent settings hub.
import { askForKey } from "./credential-commands.js";
import { keyFor } from "./credentials.js";
import { of } from "./tui/editor.js";
import { heading } from "./tui/picker.js";
import { OLLAMA_CLOUD_HOST, OLLAMA_LOCAL_HOST, parseOllamaEndpoint, } from "./providers/ollama-endpoint.js";
import { configureOllama, ollamaConnection } from "./providers/ollama.js";
const KEY = "OLLAMA_API_KEY";
export function ollamaConnectionHint() {
    const connection = ollamaConnection();
    const origin = new URL(connection.baseUrl).host;
    const automatic = connection.inferred ? " · automatic" : "";
    if (connection.kind === "local")
        return `local · this computer${automatic}`;
    return `${connection.kind} · ${origin}${automatic}`;
}
export async function ollamaConnectionSetting(session, host, persist) {
    if (host.choose === undefined)
        return;
    const current = ollamaConnection();
    const index = await host.choose({
        title: heading("Ollama connection", "where Ollama requests run", session.palette),
        right: "↑↓ enter · esc back",
        options: [
            { label: "cloud", hint: "ollama.com · API key" },
            { label: "local", hint: "this computer · no API key" },
            { label: "custom", hint: "HTTPS or loopback endpoint" },
        ],
        index: current.kind === "cloud" ? 0 : current.kind === "local" ? 1 : 2,
    });
    if (index === undefined)
        return;
    let nextHost;
    if (index === 0)
        nextHost = OLLAMA_CLOUD_HOST;
    else if (index === 1)
        nextHost = OLLAMA_LOCAL_HOST;
    else {
        const custom = await customEndpoint(session, host, current.kind === "custom" ? current.baseUrl : "https://");
        if (custom === undefined)
            return;
        nextHost = custom;
    }
    const endpoint = parseOllamaEndpoint(nextHost);
    if (!endpoint.loopback && keyFor(KEY) === undefined) {
        const accepted = await askForKey(KEY, host, session.palette);
        if (!accepted)
            return;
    }
    if (!(await persist({ ollamaHost: endpoint.baseUrl })))
        return;
    configureOllama(endpoint.baseUrl);
    session.config.ollamaHost = endpoint.baseUrl;
}
async function customEndpoint(session, host, initial) {
    if (host.type === undefined)
        return undefined;
    const field = {
        title: heading("Ollama endpoint", "HTTPS or exact loopback URL", session.palette),
        right: "enter save · esc back",
        editor: of(initial),
        secret: false,
        note: "Remote endpoints must use HTTPS. API keys are stored separately.",
    };
    const value = await host.type(field);
    if (value === undefined)
        return undefined;
    try {
        return parseOllamaEndpoint(value).baseUrl;
    }
    catch (error) {
        host.emit({ kind: "notice", text: error.message, tone: "error" });
        return undefined;
    }
}
