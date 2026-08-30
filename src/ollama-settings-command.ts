// Ollama connection choices inside the persistent settings hub.

import type { Host } from "./commands.ts";
import { askForKey } from "./credential-commands.ts";
import { keyFor } from "./credentials.ts";
import type { Session } from "./session.ts";
import type { SavedSettings } from "./settings.ts";
import { of } from "./tui/editor.ts";
import type { Field } from "./tui/field.ts";
import { heading } from "./tui/picker.ts";
import {
  OLLAMA_CLOUD_HOST,
  OLLAMA_LOCAL_HOST,
  parseOllamaEndpoint,
} from "./providers/ollama-endpoint.ts";
import { configureOllama, ollamaConnection } from "./providers/ollama.ts";

const KEY = "OLLAMA_API_KEY";

type Persist = (patch: Partial<SavedSettings>) => Promise<boolean>;

export function ollamaConnectionHint(): string {
  const connection = ollamaConnection();
  const origin = new URL(connection.baseUrl).host;
  const automatic = connection.inferred ? " · automatic" : "";
  if (connection.kind === "local") return `local · this computer${automatic}`;
  return `${connection.kind} · ${origin}${automatic}`;
}

export async function ollamaConnectionSetting(
  session: Session,
  host: Host,
  persist: Persist,
): Promise<void> {
  if (host.choose === undefined) return;
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
  if (index === undefined) return;

  let nextHost: string;
  if (index === 0) nextHost = OLLAMA_CLOUD_HOST;
  else if (index === 1) nextHost = OLLAMA_LOCAL_HOST;
  else {
    const custom = await customEndpoint(session, host, current.kind === "custom" ? current.baseUrl : "https://");
    if (custom === undefined) return;
    nextHost = custom;
  }

  const endpoint = parseOllamaEndpoint(nextHost);
  if (!endpoint.loopback && keyFor(KEY) === undefined) {
    const accepted = await askForKey(KEY, host, session.palette);
    if (!accepted) return;
  }
  if (!(await persist({ ollamaHost: endpoint.baseUrl }))) return;

  configureOllama(endpoint.baseUrl);
  session.config.ollamaHost = endpoint.baseUrl;
}

async function customEndpoint(
  session: Session,
  host: Host,
  initial: string,
): Promise<string | undefined> {
  if (host.type === undefined) return undefined;
  const field: Field = {
    title: heading("Ollama endpoint", "HTTPS or exact loopback URL", session.palette),
    right: "enter save · esc back",
    editor: of(initial),
    secret: false,
    note: "Remote endpoints must use HTTPS. API keys are stored separately.",
  };
  const value = await host.type(field);
  if (value === undefined) return undefined;
  try {
    return parseOllamaEndpoint(value).baseUrl;
  } catch (error) {
    host.emit({ kind: "notice", text: (error as Error).message, tone: "error" });
    return undefined;
  }
}
