// OpenAI Responses API, spoken directly.
//
// Responses wire contract verified against the official API reference on
// 2026-08-29. Keep final response events authoritative over display deltas.
import { postSse } from "./http.js";
import { listModels } from "./catalog.js";
import { keyFor } from "../credentials.js";
import { assembleOpenAI } from "./openai-stream.js";
import { fromWireResponse, normalizeEffort, stopNotice, toWireItems, toWireTool, } from "./openai-wire.js";
const ENDPOINT = "https://api.openai.com/v1/responses";
const MODELS = "https://api.openai.com/v1/models";
const KEY = "OPENAI_API_KEY";
/**
 * What this account can reach that is not a chat model.
 *
 * The list is an exclusion rather than an allow-list on purpose: an unknown
 * `gpt-`something is far more likely to be a model worth offering than one
 * worth hiding, and an allow-list would quietly bury every family shipped
 * after this line was written.
 */
const NOT_CHAT = /^(text-|tts-|whisper|dall-e|sora|gpt-image|omni-moderation|davinci|babbage)/;
const NON_TEXT_MODE = /(?:^|[-_])(audio|realtime|transcribe|tts)(?:[-_]|$)/;
export const openai = {
    id: "openai",
    defaultModel: "gpt-5",
    keyVar: KEY,
    blocked() {
        return apiKey() === undefined ? `${KEY} is not set` : undefined;
    },
    // The endpoint answers in no order worth keeping, so descending puts the
    // highest-numbered family — usually the newest — at the top of the menu.
    async models(signal, onStatus) {
        const ids = await listModels(MODELS, headers(requireKey()), signal, onStatus);
        return ids
            .filter((id) => !NOT_CHAT.test(id) && !NON_TEXT_MODE.test(id))
            .sort((a, b) => b.localeCompare(a));
    },
    location: () => "cloud",
    async send(req) {
        const key = requireKey();
        const events = await postSse(ENDPOINT, headers(key), {
            model: req.model,
            instructions: req.system,
            input: req.messages.flatMap(toWireItems),
            tools: req.tools.map(toWireTool),
            max_output_tokens: req.maxTokens,
            reasoning: { effort: normalizeEffort(req.effort), summary: "auto" },
            store: false,
            include: ["reasoning.encrypted_content"],
            stream: true,
        }, req.signal, req.onStatus);
        const data = await assembleOpenAI(events, req.onStream);
        const notice = stopNotice(data);
        if (notice !== undefined)
            req.onStream?.({ kind: "text", text: `\n${notice}` });
        return fromWireResponse(data);
    },
};
function apiKey() {
    return keyFor(KEY);
}
function requireKey() {
    const key = apiKey();
    if (key === undefined)
        throw new Error(`${KEY} is not set`);
    return key;
}
function headers(key) {
    return { authorization: `Bearer ${key}` };
}
