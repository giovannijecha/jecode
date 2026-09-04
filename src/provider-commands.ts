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
import { providerLabel, providerRouteLabel } from "./provider-label.ts";
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

  const groups = providerGroups();
  let selected = Math.max(
    0,
    groups.findIndex((group) => group.providers.some((provider) => provider.id === session.provider.id)),
  );
  while (true) {
    const index = await choose({
      title: [],
      description: `Current route: ${providerRouteLabel(session.provider)} · access changes do not switch it; use /models`,
      options: groups.map((group) => ({
        label: group.label,
        value: providerGroupHint(group, session.provider.id),
      })),
      index: selected,
    });
    if (index === undefined) {
      throwIfAborted(host.signal);
      return;
    }
    const group = groups[index];
    if (group === undefined) return;
    selected = index;
    await providerGroupCommand(group, session, host);
    throwIfAborted(host.signal);
  }
}

type ProviderGroup = {
  label: "Account" | "API";
  providers: readonly Provider[];
};

function providerGroups(): readonly ProviderGroup[] {
  return [
    { label: "Account", providers: PROVIDERS.filter((provider) => provider.auth.kind === "oauth") },
    { label: "API", providers: PROVIDERS.filter((provider) => provider.auth.kind !== "oauth") },
  ];
}

async function providerGroupCommand(
  group: ProviderGroup,
  session: Session,
  host: Host,
): Promise<void> {
  if (host.choose === undefined) return;
  let selected = Math.max(
    0,
    group.providers.findIndex((provider) => provider.id === session.provider.id),
  );

  while (true) {
    const index = await host.choose({
      title: heading(group.label, "provider access", session.palette),
      options: group.providers.map((provider) => ({
        label: providerLabel(provider.id),
        value: `${provider.id === session.provider.id ? "current · " : ""}${providerAccessHint(provider)}`,
      })),
      index: selected,
    });
    if (index === undefined) {
      throwIfAborted(host.signal);
      return;
    }
    const provider = group.providers[index];
    if (provider === undefined) return;
    selected = index;
    await manageProvider(provider, session, host);
    // Esc closes only the provider-specific flow. Ctrl+C also settles that
    // interaction, but aborts the command signal and must not reopen a menu.
    throwIfAborted(host.signal);
  }
}

function providerCount(count: number): string {
  return `${count} ${count === 1 ? "provider" : "providers"}`;
}

function providerGroupHint(group: ProviderGroup, activeId: string): string {
  const active = group.providers.find((provider) => provider.id === activeId);
  const count = providerCount(group.providers.length);
  return active === undefined ? count : `${count} · current ${providerRouteLabel(active)}`;
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
  await apiKeyCommand(provider.auth.keyVar, providerLabel(provider.id), session, host, provider.id);
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
    await apiKeyCommand(OLLAMA_KEY, "Ollama", session, host, "ollama");
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
