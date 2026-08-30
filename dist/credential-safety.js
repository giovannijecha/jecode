// The shell needs a useful process environment, not the application's secrets.
import { credentialValues } from "./credentials.js";
const REDACTED = "[credential redacted]";
const SENSITIVE_ENVIRONMENT_NAME = /(?:^|_)(?:API_?KEY|ACCESS_?KEY|PRIVATE_?KEY|KEY|TOKEN|SECRET|PASSWORD|PASSWD|PASS|PWD|CREDENTIALS?|AUTH|JWT|COOKIE|PAT)(?:_|$)/i;
const COMPACT_SENSITIVE_ENVIRONMENT_NAME = /^(?:PGPASSWORD)$/i;
/** Preserve ordinary tool configuration while withholding credential-like values. */
export function shellEnvironment(source = process.env) {
    const environment = {};
    for (const [name, value] of Object.entries(source)) {
        if (value !== undefined && !sensitiveEnvironment(name, value))
            environment[name] = value;
    }
    return environment;
}
/** Remove values Jecode recognizes as credentials before tool output leaves the shell boundary. */
export function redactCredentials(text, source = process.env) {
    return redact(text, secrets(source));
}
/** Redact before bounded capture, retaining enough raw overlap for split values. */
export function credentialRedactor(source = process.env) {
    const values = secrets(source);
    const longest = Math.max(0, ...values.map((value) => value.length));
    let pending = "";
    return {
        write(chunk) {
            const combined = `${pending}${chunk}`;
            const ready = [];
            let at = 0;
            while (at < combined.length) {
                const complete = values.find((value) => combined.startsWith(value, at));
                if (complete !== undefined) {
                    ready.push(REDACTED);
                    at += complete.length;
                    continue;
                }
                const rest = combined.slice(at);
                if (rest.length < longest && values.some((value) => value.startsWith(rest))) {
                    pending = rest;
                    return ready.join("");
                }
                ready.push(combined[at]);
                at++;
            }
            pending = "";
            return ready.join("");
        },
        end() {
            const last = redact(pending, values);
            pending = "";
            return last;
        },
    };
}
function sensitiveEnvironmentName(name) {
    const normalized = name
        .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
        .replace(/[^a-z0-9]+/gi, "_")
        .replace(/^_+|_+$/g, "");
    return (SENSITIVE_ENVIRONMENT_NAME.test(normalized) ||
        COMPACT_SENSITIVE_ENVIRONMENT_NAME.test(normalized));
}
function secrets(source) {
    const values = new Set(credentialValues());
    for (const [name, value] of Object.entries(source)) {
        if (value !== undefined && value !== "" && sensitiveEnvironment(name, value))
            values.add(value);
    }
    return [...values].filter((value) => value !== "").sort((left, right) => right.length - left.length);
}
function redact(text, values) {
    let redacted = text;
    for (const value of values)
        redacted = redacted.replaceAll(value, REDACTED);
    return redacted;
}
function sensitiveEnvironment(name, value) {
    if (sensitiveEnvironmentName(name))
        return true;
    try {
        const url = new URL(value);
        return url.username !== "" || url.password !== "";
    }
    catch {
        return /(?:^|[;\s])(?:password|pwd|token|secret)\s*=\s*[^;\s]+/i.test(value);
    }
}
