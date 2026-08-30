// Turning OLLAMA_HOST into one safe, normalized endpoint.
export const DEFAULT_OLLAMA_HOST = "https://ollama.com";
export function parseOllamaEndpoint(value) {
    let url;
    try {
        url = new URL(value.trim());
    }
    catch {
        throw new Error("OLLAMA_HOST must be an absolute HTTP(S) URL");
    }
    if (url.protocol !== "http:" && url.protocol !== "https:") {
        throw new Error("OLLAMA_HOST must use HTTP or HTTPS");
    }
    if (url.username !== "" || url.password !== "") {
        throw new Error("OLLAMA_HOST must not contain credentials");
    }
    if (url.search !== "" || url.hash !== "") {
        throw new Error("OLLAMA_HOST must not contain a query or fragment");
    }
    const loopback = isExactLoopback(url.hostname);
    if (url.protocol === "http:" && !loopback) {
        throw new Error("OLLAMA_HOST must use HTTPS unless it is an exact loopback address");
    }
    const pathname = url.pathname.replace(/\/+$/, "");
    return {
        baseUrl: `${url.origin}${pathname === "" ? "" : pathname}`,
        loopback,
    };
}
function isExactLoopback(hostname) {
    return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]";
}
