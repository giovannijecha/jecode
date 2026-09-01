// Provider access and connection management. Runtime selection lives in /models.

import { saveCommandSettings } from "./command-settings.ts";
import type { Host } from "./commands.ts";
import { apiKeyCommand } from "./credential-commands.ts";
import { credentialSource } from "./credentials.ts";
import {
  ollamaConnectionHint,
  ollamaConnectionSetting,
} from "./ollama-settings-command.ts";
import { openAIAccountHint } from "./openai-account.ts";
import { openAIAccountCommand } from "./openai-account-command.ts";
import { providerLabel } from "./provider-label.ts";
import { PROVIDERS } from "./providers/index.ts";
import { ollamaConnection } from "./providers/ollama.ts";
import type { Session } from "./session.ts";
import { heading } from "./tui/picker.ts";
import type { Provider } from "./types.ts";

const OLLAMA_KEY = "OLLAMA_API_KEY";

/** The single control plane for API keys, OAuth accounts, and Ollama endpoints. */
export async function providersCommand(session: Session, host: Host): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;

  let selected = Math.max(
    0,
    PROVIDERS.findIndex((provider) => provider.id === session.provider.id),
  );
  while (true) {
    const index = await choose({
      title: [],
      options: PROVIDERS.map((provider) => ({
        label: providerLabel(provider.id),
        value: providerAccessHint(provider),
      })),
      index: selected,
    });
    if (index === undefined) return;
    const provider = PROVIDERS[index];
    if (provider === undefined) return;
    selected = index;
    await manageProvider(provider, session, host);
    // Esc closes only the nested provider flow. Ctrl+C also settles that
    // picker, but aborts the command signal and must not reopen the parent.
    throwIfAborted(host.signal);
  }
}

export function providerAccessHint(provider: Provider): string {
  if (provider.id === "ollama") {
    const connection = ollamaConnection();
    if (connection.loopback) return `${ollamaConnectionHint()} · no key needed`;
    return `${ollamaConnectionHint()} · API key ${credentialSource(OLLAMA_KEY) ?? "missing"}`;
  }
  if (provider.auth.kind === "oauth") return openAIAccountHint();
  return `API key · ${credentialSource(provider.auth.keyVar) ?? "missing"}`;
}

async function manageProvider(
  provider: Provider,
  session: Session,
  host: Host,
): Promise<void> {
  if (provider.id === "ollama") {
    await ollamaProviderCommand(session, host);
    return;
  }
  if (provider.auth.kind === "oauth") {
    await openAIAccountCommand(session, host);
    return;
  }
  await apiKeyCommand(provider.auth.keyVar, providerLabel(provider.id), session, host);
}

async function ollamaProviderCommand(session: Session, host: Host): Promise<void> {
  if (host.choose === undefined) return;
  const index = await host.choose({
    title: heading("Ollama", "access and connection", session.palette),
    options: [
      { label: "connection", hint: ollamaConnectionHint() },
      { label: "API key", hint: credentialSource(OLLAMA_KEY) ?? "missing" },
    ],
    index: 0,
  });
  if (index === 0) {
    await ollamaConnectionSetting(
      session,
      host,
      (patch) => saveCommandSettings(host, patch),
    );
  } else if (index === 1) {
    await apiKeyCommand(OLLAMA_KEY, "Ollama", session, host);
  }
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("interrupted");
}
