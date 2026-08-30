// The shell needs a useful process environment, not the application's secrets.

import { credentialValues } from "./credentials.ts";

const REDACTED = "[credential redacted]";
const SENSITIVE_ENVIRONMENT_NAME =
  /(?:^|_)(?:API_?KEY|ACCESS_?KEY|PRIVATE_?KEY|KEY|TOKEN|SECRET|PASSWORD|PASSWD|PASS|CREDENTIALS?|AUTH|JWT|COOKIE)(?:_|$)/i;

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
  return SENSITIVE_ENVIRONMENT_NAME.test(name);
}

function secrets(source: NodeJS.ProcessEnv): string[] {
  const values = new Set(credentialValues());
  for (const [name, value] of Object.entries(source)) {
    if (value !== undefined && value !== "" && sensitiveEnvironment(name, value)) values.add(value);
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
