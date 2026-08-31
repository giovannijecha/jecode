// Credential command flows shared by settings and provider selection.

import type { Session } from "./session.ts";
import type { Host } from "./commands.ts";
import type { Option } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";
import type { Field } from "./tui/field.ts";
import { EMPTY } from "./tui/editor.ts";
import { PROVIDERS } from "./providers/index.ts";
import { openAIAccountHint } from "./openai-account.ts";
import {
  ensureOpenAIAccount,
  openAIAccountCommand,
} from "./openai-account-command.ts";
import {
  credentialSource,
  forgetSaved,
  hasSaved,
  hold,
  keep,
  storeLabel,
} from "./credentials.ts";
import type { Palette } from "./ui/theme.ts";

/** Ask for a key, then ask separately whether it may be written to disk. */
export async function askForKey(name: string, host: Host, pal: Palette): Promise<boolean> {
  if (host.type === undefined || host.choose === undefined) return false;

  const field: Field = {
    title: [
      { text: "paste key  ", fg: pal.ink.attention, bold: true },
      { text: name, fg: pal.ink.bright, bold: true },
    ],
    right: "enter ok · esc skip",
    editor: EMPTY,
    secret: true,
    note: "read from the environment first — an exported value still wins",
  };

  const value = await host.type(field);
  if (value === undefined) return false;

  const index = await host.choose({
    title: heading("remember it?", name, pal),
    options: [
      { label: "just this session", hint: "nothing is written", key: "s" },
      { label: `save it to ${storeLabel()}`, hint: "owner-only file", key: "w" },
      { label: "discard it", key: "d" },
    ],
    index: 0,
  });

  if (index === 0) {
    hold(name, value);
    host.emit({ kind: "notice", text: "API key ready · this session", tone: "info" });
    return true;
  }

  if (index === 1) {
    try {
      await keep(name, value);
      host.emit({ kind: "notice", text: "API key saved", tone: "info" });
      return true;
    } catch (error) {
      host.emit({ kind: "notice", text: `could not save API key · ${(error as Error).message}`, tone: "error" });
      return false;
    }
  }

  return false;
}

export async function credentialsCommand(session: Session, host: Host): Promise<void> {
  const choose = chooser(host);
  if (choose === undefined) return;

  const index = await choose({
    title: heading("authentication", "secrets are never shown", session.palette),
    options: PROVIDERS.map((provider) => ({
      label: provider.auth.kind === "api-key" ? provider.auth.keyVar : `${provider.auth.label} account`,
      hint: provider.auth.kind === "api-key"
        ? credentialSource(provider.auth.keyVar) ?? "missing"
        : openAIAccountHint(),
    })),
    index: Math.max(0, PROVIDERS.findIndex((provider) => provider.id === session.provider.id)),
  });
  if (index === undefined) return;

  const provider = PROVIDERS[index];
  if (provider === undefined) return;
  if (provider.auth.kind === "oauth") {
    await openAIAccountCommand(session, host);
    return;
  }
  const name = provider.auth.keyVar;
  const source = credentialSource(name);

  if (source === "environment") {
    host.emit({
      kind: "notice",
      text: `${name} comes from the environment · restart after changing it`,
      tone: "info",
    });
    if (hasSaved(name)) await offerForget(name, session, host, "a saved copy is currently shadowed");
    return;
  }

  const actions: Option[] = [
    { label: source === undefined ? "add credential" : "replace credential", key: "r" },
    ...(hasSaved(name) ? [{ label: "forget saved copy", hint: storeLabel(), key: "f" }] : []),
  ];
  const action = await choose({
    title: heading(name, source ?? "missing", session.palette),
    options: actions,
    index: 0,
  });
  if (action === undefined) return;
  if (actions[action]?.key === "f") {
    await forget(name, host);
    return;
  }
  await askForKey(name, host, session.palette);
}

/** Offer the authentication flow owned by a provider, if that is its blocker. */
export async function ensureProviderAuthentication(
  provider: Session["provider"],
  session: Session,
  host: Host,
): Promise<boolean> {
  const blocked = provider.blocked();
  if (blocked === undefined) return true;
  if (provider.auth.kind === "oauth") {
    return provider.auth.account === "openai-codex"
      ? ensureOpenAIAccount(session, host)
      : false;
  }
  if (!blocked.startsWith(`${provider.auth.keyVar} `)) {
    host.emit({ kind: "notice", text: blocked, tone: "error" });
    return false;
  }
  await askForKey(provider.auth.keyVar, host, session.palette);
  return provider.blocked() === undefined;
}

export function authenticationNeed(provider: Session["provider"]): string {
  return provider.auth.kind === "oauth" ? `${provider.auth.label} sign-in` : "an API key";
}

async function offerForget(name: string, session: Session, host: Host, hint: string): Promise<void> {
  if (host.choose === undefined) return;
  const index = await host.choose({
    title: heading(name, hint, session.palette),
    options: [
      { label: "keep saved copy", key: "k" },
      { label: "forget saved copy", hint: storeLabel(), key: "f" },
    ],
    index: 0,
  });
  if (index === 1) await forget(name, host);
}

async function forget(name: string, host: Host): Promise<void> {
  try {
    const removed = await forgetSaved(name);
    host.emit({
      kind: "notice",
      text: removed ? "API key removed" : "no saved API key",
      tone: removed ? "info" : "warn",
    });
  } catch (error) {
    host.emit({ kind: "notice", text: `could not remove API key · ${(error as Error).message}`, tone: "error" });
  }
}

function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}
