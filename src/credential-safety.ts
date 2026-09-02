// The shell needs a useful process environment, not the application's secrets.

import { credentialValues } from "./credentials.ts";
import { accountValues } from "./accounts.ts";

const REDACTED = "[credential redacted]";
const MIN_HEURISTIC_SECRET_CHARS = 8;
const EXPLICIT_CREDENTIAL_ENVIRONMENT_NAMES = new Set([
  "ANTHROPIC_API_KEY",
  "OLLAMA_API_KEY",
  "OPENAI_API_KEY",
]);
const SENSITIVE_ENVIRONMENT_NAME =
  /(?:^|_)(?:API_?KEY|ACCESS_?KEY|PRIVATE_?KEY|KEY|TOKEN|SECRET|PASSWORD|PASSWD|PASS|PWD|CREDENTIALS?|AUTH|JWT|COOKIE|PAT)(?:_|$)/i;
const COMPACT_SENSITIVE_ENVIRONMENT_NAME = /^(?:PGPASSWORD)$/i;
const SAFE_ENVIRONMENT_NAMES = new Set([
  "SSH_AUTH_SOCK",
  "PWD",
  "OLDPWD",
  "PASSWORD_STORE_DIR",
]);

/** Preserve ordinary tool configuration while withholding credential-like values. */
export function shellEnvironment(source: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const environment: NodeJS.ProcessEnv = {};
  for (const [name, value] of Object.entries(source)) {
    if (value !== undefined && !sensitiveEnvironment(name, value)) environment[name] = value;
  }
  return environment;
}

/** Remove values Jecode recognizes as credentials before tool output leaves the shell boundary. */
export function redactCredentials(text: string, source: NodeJS.ProcessEnv = process.env): string {
  return redact(text, secrets(source));
}

/** Redact before bounded capture, retaining enough raw overlap for split values. */
export function credentialRedactor(source: NodeJS.ProcessEnv = process.env): {
  write(chunk: string): string;
  end(): string;
} {
  const values = secrets(source);
  const longest = Math.max(0, ...values.map((value) => value.length));
  let pending = "";

  return {
    write(chunk) {
      const combined = `${pending}${chunk}`;
      const ready: string[] = [];
      let at = 0;
      while (at < combined.length) {
        const rest = combined.slice(at);
        // A complete shorter credential can also be the prefix of a longer
        // one. Hold that ambiguous suffix until the next chunk proves which
        // value arrived, otherwise the longer credential leaks its tail.
        if (
          rest.length < longest &&
          values.some((value) => value.length > rest.length && value.startsWith(rest))
        ) {
          pending = rest;
          return ready.join("");
        }
        const complete = values.find((value) => combined.startsWith(value, at));
        if (complete !== undefined) {
          ready.push(REDACTED);
          at += complete.length;
          continue;
        }
        ready.push(combined[at] as string);
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

function sensitiveEnvironmentName(name: string): boolean {
  const normalized = name
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/[^a-z0-9]+/gi, "_")
    .replace(/^_+|_+$/g, "");
  if (SAFE_ENVIRONMENT_NAMES.has(normalized.toUpperCase())) return false;
  return (
    SENSITIVE_ENVIRONMENT_NAME.test(normalized) ||
    COMPACT_SENSITIVE_ENVIRONMENT_NAME.test(normalized)
  );
}

function secrets(source: NodeJS.ProcessEnv): string[] {
  const values = new Set([...credentialValues(), ...accountValues()]);
  for (const [name, value] of Object.entries(source)) {
    if (value === undefined || value === "") continue;
    const explicit = EXPLICIT_CREDENTIAL_ENVIRONMENT_NAMES.has(name.toUpperCase());
    if (explicit || (value.length >= MIN_HEURISTIC_SECRET_CHARS && sensitiveEnvironment(name, value))) {
      values.add(value);
    }
  }
  return [...values].filter((value) => value !== "").sort((left, right) => right.length - left.length);
}

function redact(text: string, values: readonly string[]): string {
  let redacted = text;
  for (const value of values) redacted = redacted.replaceAll(value, REDACTED);
  return redacted;
}

function sensitiveEnvironment(name: string, value: string): boolean {
  if (sensitiveEnvironmentName(name)) return true;
  try {
    const url = new URL(value);
    return url.username !== "" || url.password !== "";
  } catch {
    return /(?:^|[;\s])(?:password|pwd|token|secret)\s*=\s*[^;\s]+/i.test(value);
  }
}
