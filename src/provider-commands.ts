// Provider and model command flows.

import type { Session } from "./session.ts";
import type { Host } from "./commands.ts";
import type { Option } from "./tui/picker.ts";
import { heading } from "./tui/picker.ts";
import { PROVIDERS } from "./providers/index.ts";
import { readSettings } from "./settings.ts";
import type { SavedSettings } from "./settings.ts";
import {
  authenticationNeed,
  ensureProviderAuthentication,
} from "./credential-commands.ts";
import { providerFailure } from "./provider-errors.ts";
import { providerLabel } from "./provider-label.ts";

type SelectionBehavior = {
  announce?: boolean;
  save?: boolean;
};

/**
 * The provider menu.
 *
 * Every provider is offered, including the ones that cannot run: the reason
 * one is unusable — the variable it wants, by name — is worth more on screen
 * than the row would be worth hidden. A blocked choice opens the same masked
 * credential flow used by settings, and cancellation leaves the old choice.
 */
export async function providersCommand(
  session: Session,
  host: Host,
  behavior: SelectionBehavior = {},
): Promise<boolean> {
  const choose = chooser(host);
  if (choose === undefined) return false;

  const options: Option[] = PROVIDERS.map((provider) => ({
    label: provider.id,
    hint: provider.blocked() ?? "ready",
  }));

  const at = PROVIDERS.findIndex((provider) => provider.id === session.provider.id);
  const index = await choose({
    title: heading("provider", "where the next turn runs", session.palette),
    options,
    index: Math.max(0, at),
  });
  if (index === undefined) return false;

  const chosen = PROVIDERS[index];
  if (chosen === undefined) return false;

  // Picking the provider already in use is not a no-op when it cannot run:
  // it is how the user asks to fix the reason it cannot.
  if (chosen.id === session.provider.id) {
    if (!(await ensureProviderAuthentication(chosen, session, host))) return false;
    return session.model === ""
      ? modelsCommand(session, host, { announce: false, save: behavior.save })
      : true;
  }

  // A provider that cannot run is worth one offer to fix it, here, rather
  // than a note telling the user to leave and export something.
  const blocked = chosen.blocked();
  if (blocked !== undefined) {
    await ensureProviderAuthentication(chosen, session, host);
    const still = chosen.blocked();
    if (still !== undefined) {
      host.emit({
        kind: "notice",
        text: `${providerLabel(chosen.id)} still needs ${authenticationNeed(chosen)} · provider unchanged`,
        tone: "warn",
      });
      return false;
    }
  }

  const before = {
    provider: session.provider,
    model: session.model,
    providerId: session.config.providerId,
    configModel: session.config.model,
  };
  session.provider = chosen;
  // The model belonged to the old provider. Carrying it across would send
  // `claude-sonnet-5` to OpenAI and call the 404 a bug in the provider.
  session.model = readSettings().models?.[chosen.id] ?? chosen.defaultModel;
  session.config.providerId = chosen.id;
  session.config.model = session.model;

  if (session.model === "" && !(await modelsCommand(session, host, { announce: false, save: false }))) {
    session.provider = before.provider;
    session.model = before.model;
    session.config.providerId = before.providerId;
    session.config.model = before.configModel;
    return false;
  }

  if (behavior.save !== false) {
    const saved = readSettings();
    const models = { ...saved.models };
    if (session.model !== "") models[chosen.id] = session.model;
    if (!(await saveDefaults(host, { provider: chosen.id, models }))) {
      session.provider = before.provider;
      session.model = before.model;
      session.config.providerId = before.providerId;
      session.config.model = before.configModel;
      return false;
    }
  }

  if (behavior.announce !== false) {
    host.emit({
      kind: "notice",
      text: session.model === "" ? `provider · ${chosen.id} · pick a model` : `provider · ${chosen.id}`,
      tone: "info",
    });
  }
  return true;
}

/** The model menu, built from what the provider says it has right now. */
export async function modelsCommand(
  session: Session,
  host: Host,
  behavior: SelectionBehavior = {},
): Promise<boolean> {
  // Before the network, not after: asking a provider for a list nothing can
  // be picked from is a request spent on a menu that will never open.
  const choose = chooser(host);
  if (choose === undefined) return false;

  const provider = session.provider;

  // Offer the key rather than only naming what is missing: the user came here
  // to pick a model, and "go and export a variable" is not an answer.
  const blocked = provider.blocked();
  if (blocked !== undefined) {
    await ensureProviderAuthentication(provider, session, host);
    const still = provider.blocked();
    if (still !== undefined) {
      host.emit({
        kind: "notice",
        text: `${providerLabel(provider.id)} still needs ${authenticationNeed(provider)}`,
        tone: "warn",
      });
      return false;
    }
  }

  host.status?.(`Asking ${provider.id}`);
  let ids: string[];
  try {
    ids = await provider.models(host.signal, (status) => host.status?.(status));
  } catch (error) {
    host.emit({
      kind: "notice",
      text: providerFailure(provider, error as Error, true),
      tone: "error",
    });
    return false;
  } finally {
    host.status?.(undefined);
  }

  if (ids.length === 0) {
    host.emit({ kind: "notice", text: `${provider.id} offers no models`, tone: "warn" });
    return false;
  }

  const index = await choose({
    title: heading("model", provider.id, session.palette),
    options: ids.map((id) => ({ label: id })),
    searchable: true,
    query: "",
    index: Math.max(0, ids.indexOf(session.model)),
  });
  if (index === undefined) return false;

  const chosen = ids[index];
  if (chosen === undefined) return false;

  const before = { model: session.model, configModel: session.config.model };
  session.model = chosen;
  session.config.model = chosen;
  if (behavior.save !== false) {
    const saved = readSettings();
    const models = { ...saved.models, [provider.id]: chosen };
    if (!(await saveDefaults(host, { models }))) {
      session.model = before.model;
      session.config.model = before.configModel;
      return false;
    }
  }
  if (behavior.announce !== false) {
    host.emit({ kind: "notice", text: `model · ${chosen}`, tone: "info" });
  }
  return true;
}

/** The way to put a menu up, or nothing — and the reason, already said. */
function chooser(host: Host): Host["choose"] {
  if (host.choose === undefined) {
    host.emit({ kind: "notice", text: "that command needs the screen", tone: "warn" });
  }
  return host.choose;
}

async function saveDefaults(host: Host, patch: Partial<SavedSettings>): Promise<boolean> {
  if (host.saveSettings === undefined) return true;
  try {
    await host.saveSettings(patch);
    return true;
  } catch (error) {
    host.emit({
      kind: "notice",
      text: `could not save settings · ${(error as Error).message}`,
      tone: "error",
    });
    return false;
  }
}
