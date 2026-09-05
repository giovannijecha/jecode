// Loopback callback used by the browser OAuth flow.

import { timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import type { IncomingMessage, Server, ServerResponse } from "node:http";
import { leadingText } from "./text-boundary.ts";
import { providerLabel } from "./provider-label.ts";
import { oauthResultPage } from "./oauth-result-page.ts";

const ACCOUNT_LABEL = providerLabel("openai-codex");
const CALLBACK_PORTS = [1455, 1457] as const;
export const OPENAI_CALLBACK_PATH = "/auth/callback";

export type OpenAICallback = {
  port: number;
  code: Promise<string>;
  respond(success: boolean): Promise<void>;
  close(): Promise<void>;
};

export async function openAICallback(state: string): Promise<OpenAICallback> {
  let resolveCode: (code: string) => void = () => {};
  let rejectCode: (error: Error) => void = () => {};
  const code = new Promise<string>((resolve, reject) => {
    resolveCode = resolve;
    rejectCode = reject;
  });
  // A bad callback can arrive before the TUI starts awaiting `complete()`.
  // Mark the rejection handled here while preserving it for the real waiter.
  void code.catch(() => undefined);
  let response: ServerResponse | undefined;
  let finished = false;

  const handler = (request: IncomingMessage, outgoing: ServerResponse): void => {
    if (finished) {
      outgoing.writeHead(409).end("Sign-in already completed.");
      return;
    }
    if (request.method !== "GET" || request.url === undefined || request.url.length > 4_096) {
      outgoing.writeHead(400).end("Invalid sign-in callback.");
      return;
    }
    let incoming: URL;
    try {
      incoming = new URL(request.url, "http://localhost");
    } catch {
      outgoing.writeHead(400).end("Invalid sign-in callback.");
      return;
    }
    if (incoming.pathname !== OPENAI_CALLBACK_PATH) {
      outgoing.writeHead(404).end("Not found.");
      return;
    }

    const receivedState = incoming.searchParams.get("state");
    // A stale browser tab or another local client can reach the fixed callback
    // port. Reject that one request without letting it terminate this login.
    if (!sameState(state, receivedState)) {
      outgoing.writeHead(400, { "cache-control": "no-store" }).end("Invalid sign-in state.");
      return;
    }

    finished = true;
    response = outgoing;
    const authError = incoming.searchParams.get("error_description") ?? incoming.searchParams.get("error");
    const authorizationCode = incoming.searchParams.get("code");
    if (authError !== null) {
      rejectCode(new Error(`${ACCOUNT_LABEL} sign-in was rejected · ${leadingText(authError, 300)}`));
    } else if (authorizationCode === null || authorizationCode === "") {
      rejectCode(new Error(`${ACCOUNT_LABEL} sign-in returned no authorization code`));
    } else {
      resolveCode(authorizationCode);
    }
  };

  const listening = await firstAvailableServer(handler);
  return {
    port: listening.port,
    code,
    async respond(success) {
      if (response === undefined || response.writableEnded) return;
      const flushed = new Promise<void>((resolve) => {
        response?.once("finish", resolve);
        response?.once("close", resolve);
      });
      response.writeHead(success ? 200 : 400, {
        "cache-control": "no-store",
        connection: "close",
        "content-security-policy": "default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        "content-type": "text/html; charset=utf-8",
        "referrer-policy": "no-referrer",
        "x-content-type-options": "nosniff",
      });
      response.end(oauthResultPage(success));
      await flushed;
    },
    close: () => closeServer(listening.server),
  };
}

async function firstAvailableServer(
  handler: (request: IncomingMessage, response: ServerResponse) => void,
): Promise<{ server: Server; port: number }> {
  for (const port of CALLBACK_PORTS) {
    const server = createServer(handler);
    server.maxHeadersCount = 40;
    server.headersTimeout = 5_000;
    server.keepAliveTimeout = 500;
    server.requestTimeout = 5_000;
    try {
      await listen(server, port);
      return { server, port };
    } catch (error) {
      await closeServer(server);
      if ((error as NodeJS.ErrnoException).code !== "EADDRINUSE") throw error;
    }
  }
  throw new Error(`${ACCOUNT_LABEL} sign-in could not open callback ports 1455 or 1457`);
}

function listen(server: Server, port: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const onError = (error: Error) => reject(error);
    server.once("error", onError);
    server.listen(port, "127.0.0.1", () => {
      server.off("error", onError);
      resolve();
    });
  });
}

function closeServer(server: Server): Promise<void> {
  return new Promise((resolve) => {
    if (!server.listening) {
      resolve();
      return;
    }
    let settled = false;
    let fallback: NodeJS.Timeout | undefined;
    const finish = () => {
      if (settled) return;
      settled = true;
      if (fallback !== undefined) clearTimeout(fallback);
      resolve();
    };
    fallback = setTimeout(() => {
      server.closeAllConnections();
      finish();
    }, 500);
    server.close(finish);
    server.closeIdleConnections();
  });
}

function sameState(expected: string, received: string | null): boolean {
  if (received === null) return false;
  const left = Buffer.from(expected);
  const right = Buffer.from(received);
  return left.length === right.length && timingSafeEqual(left, right);
}
