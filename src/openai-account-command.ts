// Interactive ChatGPT account management for the OpenAI Codex provider.

import type { Host } from "./commands.ts";
import { openAICodexAccount } from "./accounts.ts";
import { headlessEnvironment, openExternal } from "./external-browser.ts";
import {
  openAIAccountHint,
  removeOpenAIAccount,
  saveOpenAIAccount,
} from "./openai-account.ts";
import {
  beginBrowserLogin,
  beginDeviceLogin,
  type OpenAILogin,
} from "./openai-oauth.ts";
import type { Session } from "./session.ts";
import { heading } from "./tui/picker.ts";

export type OpenAIAccountCommandDependencies = {
  beginBrowser(): Promise<OpenAILogin>;
  beginDevice(signal?: AbortSignal): Promise<OpenAILogin>;
  openUrl(url: string): Promise<boolean>;
  headless(): boolean;
};

const DEFAULT_DEPENDENCIES: OpenAIAccountCommandDependencies = {
  beginBrowser: beginBrowserLogin,
  beginDevice: beginDeviceLogin,
  openUrl: openExternal,
  headless: headlessEnvironment,
};

export async function openAIAccountCommand(
  session: Session,
  host: Host,
  dependencies: OpenAIAccountCommandDependencies = DEFAULT_DEPENDENCIES,
): Promise<boolean> {
  if (host.choose === undefined) return false;
  const connected = openAICodexAccount() !== undefined;
  const actions = connected
    ? [
        { label: "reconnect ChatGPT", hint: "replace the current sign-in", key: "r" },
        { label: "sign out", hint: "remove the saved account", key: "s" },
      ]
    : [{ label: "connect ChatGPT", hint: "sign in with OpenAI", key: "c" }];
  const action = await host.choose({
    title: heading("ChatGPT", openAIAccountHint(), session.palette),
    options: actions,
    index: 0,
  });
  if (action === undefined) return false;

  if (connected && actions[action]?.key === "s") {
    const result = await removeOpenAIAccount(host.signal);
    const route = session.provider.id === "openai-codex"
      ? " · choose another provider in /models"
      : "";
    host.emit({
      kind: "notice",
      text: result.revokeFailed
        ? `ChatGPT disconnected · remote sign-out could not be confirmed${route}`
        : `ChatGPT disconnected${route}`,
      tone: result.revokeFailed ? "warn" : "info",
    });
    return result.removed;
  }
  return connectOpenAIAccount(session, host, dependencies);
}

export async function ensureOpenAIAccount(
  session: Session,
  host: Host,
  dependencies: OpenAIAccountCommandDependencies = DEFAULT_DEPENDENCIES,
): Promise<boolean> {
  if (openAICodexAccount() !== undefined) return true;
  return connectOpenAIAccount(session, host, dependencies);
}

async function connectOpenAIAccount(
  session: Session,
  host: Host,
  dependencies: OpenAIAccountCommandDependencies,
): Promise<boolean> {
  if (host.choose === undefined) return false;
  const methods = [
    { label: "sign in with browser", hint: "desktop terminal", key: "b" },
    { label: "sign in with device code", hint: "WSL, SSH, or headless", key: "d" },
  ];
  const choice = await host.choose({
    title: heading("connect ChatGPT", "OpenAI OAuth", session.palette),
    description: "Choose the flow for this terminal. Your password stays on OpenAI's website.",
    options: methods,
    index: dependencies.headless() ? 1 : 0,
  });
  if (choice === undefined) return false;

  host.status?.("Starting ChatGPT sign-in");
  let login: OpenAILogin;
  try {
    login = choice === 1
      ? await dependencies.beginDevice(host.signal)
      : await dependencies.beginBrowser();
  } finally {
    host.status?.(undefined);
  }
  return waitForLogin(login, session, host, dependencies.openUrl);
}

async function waitForLogin(
  login: OpenAILogin,
  session: Session,
  host: Host,
  openUrl: (url: string) => Promise<boolean>,
): Promise<boolean> {
  if (host.choose === undefined) return false;
  const local = new AbortController();
  const signal = host.signal === undefined
    ? local.signal
    : AbortSignal.any([host.signal, local.signal]);
  const completion = login.complete(signal).then(
    (account) => ({ kind: "account" as const, account }),
    (error: unknown) => ({ kind: "error" as const, error: error as Error }),
  );

  await openUrl(login.url);
  try {
    while (true) {
      const selection = host.choose(waitingPicker(login, session));
      const outcome = await Promise.race([
        completion,
        selection.then((index) => ({ kind: "selection" as const, index })),
      ]);
      if (outcome.kind === "account") {
        host.dismiss?.();
        await saveOpenAIAccount(outcome.account, host.signal);
        host.emit({
          kind: "notice",
          text: session.provider.id === "openai-codex"
            ? "ChatGPT connected · current provider route"
            : "ChatGPT connected · choose a ChatGPT model in /models to use this account",
          tone: "info",
        });
        return true;
      }
      if (outcome.kind === "error") {
        host.dismiss?.();
        throw outcome.error;
      }
      if (outcome.index === 0) {
        await openUrl(login.url);
        continue;
      }
      local.abort(new Error("ChatGPT sign-in cancelled"));
      return false;
    }
  } finally {
    await login.close();
  }
}

function waitingPicker(login: OpenAILogin, session: Session) {
  const detail = login.code === undefined
    ? "Finish sign-in in your browser. Jecode will continue automatically."
    : `Enter ${login.code} on OpenAI's device page. Jecode will continue automatically.`;
  return {
    title: heading("ChatGPT sign-in", login.code ?? "waiting for browser", session.palette),
    description: detail,
    options: [
      { label: "open browser again", key: "o" },
      { label: "cancel sign-in", key: "c" },
    ],
    index: 0,
  };
}
