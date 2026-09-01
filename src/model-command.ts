// Model-first runtime selection across every provider that can answer now.

import type { Host } from "./commands.ts";
import { saveCommandSettings } from "./command-settings.ts";
import { compatibleEffort } from "./effort.ts";
import { providerFailure } from "./provider-errors.ts";
import { providerLabel } from "./provider-label.ts";
import { PROVIDERS } from "./providers/index.ts";
import type { Session } from "./session.ts";
import { readSettings } from "./settings.ts";
import type { Provider } from "./types.ts";

type SelectionBehavior = {
  announce?: boolean;
  save?: boolean;
};

type ModelChoice = {
  provider: Provider;
  model: string;
};

type FailedCatalog = {
  provider: Provider;
  error: Error;
};

/** Build one searchable catalogue from every provider that is usable now. */
export async function modelsCommand(
  session: Session,
  host: Host,
  behavior: SelectionBehavior = {},
  providers: readonly Provider[] = PROVIDERS,
): Promise<boolean> {
  const choose = chooser(host);
  if (choose === undefined) return false;

  const availability = providers.map((provider) => ({
    provider,
    blocked: provider.blocked(),
  }));
  const connected = availability
    .filter((entry) => entry.blocked === undefined)
    .map((entry) => entry.provider);
  const disconnected = availability
    .filter((entry) => entry.blocked !== undefined)
    .map((entry) => entry.provider);
  if (connected.length === 0) {
    host.emit({
      kind: "notice",
      text: "no providers are connected · use /providers",
      tone: "warn",
    });
    return false;
  }

  throwIfAborted(host.signal);
  host.status?.("Loading model catalogs");
  let settled: PromiseSettledResult<{ provider: Provider; models: string[] }>[];
  try {
    settled = await Promise.allSettled(
      connected.map(async (provider) => ({
        provider,
        models: await provider.models(
          host.signal,
          (status) => host.status?.(`${providerLabel(provider.id)} · ${status}`),
        ),
      })),
    );
    // allSettled deliberately preserves partial provider failures, but an
    // explicit command cancellation is not a catalogue failure.
    throwIfAborted(host.signal);
  } finally {
    host.status?.(undefined);
  }

  const choices: ModelChoice[] = [];
  const failed: FailedCatalog[] = [];
  for (let index = 0; index < settled.length; index++) {
    const result = settled[index];
    const provider = connected[index];
    if (result === undefined || provider === undefined) continue;
    if (result.status === "rejected") {
      failed.push({
        provider,
        error: result.reason instanceof Error ? result.reason : new Error(String(result.reason)),
      });
      continue;
    }
    for (const model of result.value.models) choices.push({ provider, model });
  }

  if (choices.length === 0) {
    host.emit({
      kind: "notice",
      text: emptyCatalogMessage(connected, failed),
      tone: failed.length === connected.length ? "error" : "warn",
    });
    return false;
  }

  const description = catalogDescription(disconnected, failed);
  const index = await choose({
    title: [],
    ...(description === undefined ? {} : { description }),
    options: choices.map((choice) => ({
      label: choice.model,
      // Provider identity is part of the choice, not optional help: keep it
      // visible even when the terminal is only at the supported minimum.
      value: providerLabel(choice.provider.id),
    })),
    searchable: true,
    query: "",
    index: Math.max(0, choices.findIndex(
      (choice) => choice.provider.id === session.provider.id && choice.model === session.model,
    )),
  });
  throwIfAborted(host.signal);
  const chosen = index === undefined ? undefined : choices[index];
  if (chosen === undefined) return false;

  const beforeEffort = session.config.effort;
  const alignment = await alignEffort(
    chosen.provider,
    chosen.model,
    beforeEffort,
    host,
  );
  if (!alignment.ok) return false;

  if (behavior.save !== false) {
    const saved = readSettings();
    const patch = {
      provider: chosen.provider.id,
      models: { ...saved.models, [chosen.provider.id]: chosen.model },
      ...(alignment.effort === beforeEffort ? {} : { effort: alignment.effort }),
    };
    if (!(await saveCommandSettings(host, patch))) return false;
  }

  // Commit the runtime selection only after both provider validation and
  // persistence succeed, so neither the footer nor a later turn can observe a
  // half-applied provider/model pair.
  session.provider = chosen.provider;
  session.model = chosen.model;
  session.config.providerId = chosen.provider.id;
  session.config.model = chosen.model;
  session.config.effort = alignment.effort;

  if (behavior.announce !== false) {
    host.emit({
      kind: "notice",
      text: `model · ${providerLabel(chosen.provider.id)} · ${chosen.model}`,
      tone: "info",
    });
  }
  return true;
}

type EffortAlignment =
  | { ok: true; effort: string }
  | { ok: false };

async function alignEffort(
  provider: Provider,
  model: string,
  effort: string,
  host: Host,
): Promise<EffortAlignment> {
  if (provider.efforts === undefined) return { ok: true, effort };
  try {
    throwIfAborted(host.signal);
    const supported = await provider.efforts(
      model,
      host.signal,
      (status) => host.status?.(status),
    );
    throwIfAborted(host.signal);
    return { ok: true, effort: compatibleEffort(effort, supported) ?? effort };
  } catch (error) {
    throwIfAborted(host.signal);
    host.emit({
      kind: "notice",
      text: providerFailure(provider, error as Error, true),
      tone: "error",
    });
    return { ok: false };
  }
}

function catalogDescription(
  disconnected: readonly Provider[],
  failed: readonly FailedCatalog[],
): string | undefined {
  const parts = [
    disconnected.length === 0
      ? undefined
      : `Not connected: ${disconnected.map((provider) => providerLabel(provider.id)).join(", ")}`,
    failed.length === 0
      ? undefined
      : `Unavailable: ${failed.map((entry) => providerLabel(entry.provider.id)).join(", ")}`,
  ].filter((part): part is string => part !== undefined);
  return parts.length === 0 ? undefined : `${parts.join(" · ")} · manage in /providers`;
}

function emptyCatalogMessage(
  connected: readonly Provider[],
  failed: readonly FailedCatalog[],
): string {
  if (failed.length === 1 && connected.length === 1) {
    const one = failed[0];
    if (one !== undefined) return providerFailure(one.provider, one.error, true);
  }
  if (failed.length > 0) {
    return `model catalogs unavailable: ${failed.map((entry) => providerLabel(entry.provider.id)).join(", ")} · /providers`;
  }
  return "connected providers offer no models · check /providers";
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
