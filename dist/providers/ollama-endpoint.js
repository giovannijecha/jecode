// Turning an Ollama host into one safe, normalized endpoint.
export const OLLAMA_LOCAL_HOST = "http://127.0.0.1:11434";
export const OLLAMA_CLOUD_HOST = "https://ollama.com";
export function parseOllamaEndpoint(value) {
    let url;
    try {
        url = new URL(value.trim());
    }
    catch {
        throw new Error("Ollama endpoint must be an absolute HTTP(S) URL");
    }
    if (url.protocol !== "http:" && url.protocol !== "https:") {
        throw new Error("Ollama endpoint must use HTTP or HTTPS");
    }
    if (url.username !== "" || url.password !== "") {
        throw new Error("Ollama endpoint must not contain credentials");
    }
    if (url.search !== "" || url.hash !== "") {
        throw new Error("Ollama endpoint must not contain a query or fragment");
    }
    const loopback = isExactLoopback(url.hostname);
    if (url.protocol === "http:" && !loopback) {
        throw new Error("Ollama endpoint must use HTTPS unless it is an exact loopback address");
    }
    const pathname = url.pathname.replace(/\/+$/, "");
    return {
        baseUrl: `${url.origin}${pathname === "" ? "" : pathname}`,
        loopback,
    };
}
export function ollamaConnectionKind(endpoint) {
    if (endpoint.baseUrl === OLLAMA_CLOUD_HOST)
        return "cloud";
    if (endpoint.baseUrl === OLLAMA_LOCAL_HOST)
        return "local";
    return "custom";
}
function isExactLoopback(hostname) {
    return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]";
}
