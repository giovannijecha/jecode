// Ollama, spoken through its OpenAI-compatible Chat Completions endpoint.
//
// One provider covers both deployments. An explicit session endpoint wins;
// otherwise a configured key selects Ollama Cloud and no key selects the local
// daemon. There is no default model — the catalogue is whatever the host has
// pulled or the subscription grants — so the model has to be named with
// --model.
import { postSse } from "./http.js";
import { listModels } from "./catalog.js";
import { keyFor } from "../credentials.js";
import { assembleOllama } from "./ollama-stream.js";
import { OLLAMA_CLOUD_HOST, OLLAMA_LOCAL_HOST, ollamaConnectionKind, parseOllamaEndpoint, } from "./ollama-endpoint.js";
import { fromWireReply, stopNotice, toWireMessages, toWireTool } from "./ollama-wire.js";
const KEY = "OLLAMA_API_KEY";
let configuredHost;
/** Set the endpoint selected for this process. Undefined restores key-aware inference. */
export function configureOllama(host) {
    configuredHost = host === undefined ? undefined : parseOllamaEndpoint(host).baseUrl;
}
export function ollamaConnection() {
    const inferred = configuredHost === undefined;
    const endpoint = parseOllamaEndpoint(configuredHost ?? (apiKey() === undefined ? OLLAMA_LOCAL_HOST : OLLAMA_CLOUD_HOST));
    return { ...endpoint, kind: ollamaConnectionKind(endpoint), inferred };
}
export const ollama = {
    id: "ollama",
    defaultModel: "",
    keyVar: KEY,
    // The only provider whose key is conditional: a daemon on this machine is
    // reached over loopback and asks for nothing, so demanding a key there
    // would be an invented requirement.
    blocked() {
        try {
            const at = endpoint();
            if (apiKey() !== undefined || at.loopback)
                return undefined;
            return `${KEY} is not set (required by ${at.baseUrl})`;
        }
        catch (error) {
            return error.message;
        }
    },
    // Whatever the daemon has pulled, or whatever the subscription grants.
    models(signal, onStatus) {
        const at = endpoint();
        return listModels(`${at.baseUrl}/v1/models`, headers(at), signal, onStatus);
    },
    location: () => {
        try {
            return endpoint().loopback ? "local" : "cloud";
        }
        catch {
            return "cloud";
        }
    },
    async send(req) {
        const at = endpoint();
        // `effort` has no equivalent in this shape and is dropped rather than
        // guessed at — depth on these models is a property of the model chosen.
        const events = await postSse(`${at.baseUrl}/v1/chat/completions`, headers(at), {
            model: req.model,
            messages: toWireMessages(req.system, req.messages),
            tools: req.tools.map(toWireTool),
            max_tokens: req.maxTokens,
            stream: true,
        }, req.signal, req.onStatus);
        const reply = await assembleOllama(events, req.onStream);
        const notice = stopNotice(reply);
        if (notice !== undefined)
            req.onStream?.({ kind: "text", text: `\n${notice}` });
        return fromWireReply(reply);
    },
};
function endpoint() {
    return ollamaConnection();
}
function apiKey() {
    return keyFor(KEY);
}
function headers(at) {
    if (at.loopback)
        return {};
    const key = apiKey();
    if (key !== undefined)
        return { authorization: `Bearer ${key}` };
    throw new Error(`${KEY} is not set (required by ${at.baseUrl})`);
}
