// Loopback callback used by the browser OAuth flow.

import { timingSafeEqual } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import type { IncomingMessage, Server, ServerResponse } from "node:http";
import { leadingText } from "./text-boundary.ts";

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
    const incoming = new URL(request.url, "http://localhost");
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
      rejectCode(new Error(`ChatGPT sign-in was rejected · ${leadingText(authError, 300)}`));
    } else if (authorizationCode === null || authorizationCode === "") {
      rejectCode(new Error("ChatGPT sign-in returned no authorization code"));
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
      response.end(resultPage(success));
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
  throw new Error("ChatGPT sign-in could not open callback ports 1455 or 1457");
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

function resultPage(success: boolean): string {
  const title = success ? "Signed in to Jecode" : "Jecode sign-in failed";
  const status = success ? "Authentication complete" : "Authentication stopped";
  const detail = success
    ? "Return to your terminal. Jecode will continue automatically."
    : "Return to your terminal to see what stopped the connection.";
  const state = success ? "success" : "failure";
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="dark">
  <title>${title}</title>
  <style>
    :root{--night:#000;--steel:#669bd2;--steel-soft:#8db4dd;--bright:#ebeff4;--danger:#e87070}
    *{box-sizing:border-box}
    body{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--night);color:var(--steel-soft);font-family:"Segoe UI",system-ui,sans-serif}
    main{width:min(34rem,calc(100vw - 3rem));padding:3rem 1.5rem;text-align:center}
    img{display:block;width:clamp(7.5rem,20vw,10rem);height:auto;margin:0 auto 1.75rem;filter:drop-shadow(0 1.25rem 2rem rgba(102,155,210,.16))}
    .rail{width:min(18rem,70vw);height:1px;margin:0 auto 1.5rem;background:linear-gradient(90deg,transparent,var(--steel),transparent)}
    .status{margin:0 0 .75rem;color:var(--steel);font:600 .72rem/1.2 ui-monospace,"Cascadia Mono",monospace;letter-spacing:.14em;text-transform:uppercase}
    h1{margin:0;color:var(--steel);font-size:clamp(2rem,6vw,3.25rem);font-weight:720;letter-spacing:-.04em;line-height:1.05}
    p:last-of-type{max-width:30rem;margin:1.25rem auto 0;color:var(--steel-soft);font-size:1.05rem;line-height:1.6}
    .failure h1,.failure .status{color:var(--danger)}
    @media (prefers-reduced-motion:no-preference){main{animation:arrive .45s ease-out both}@keyframes arrive{from{opacity:0;transform:translateY(.6rem)}to{opacity:1;transform:none}}}
  </style>
</head>
<body>
  <main class="${state}">
    <img src="${mascotDataUri()}" alt="Jeco, the Jecode gecko">
    <div class="rail" aria-hidden="true"></div>
    <p class="status">${status}</p>
    <h1>${title}</h1>
    <p>${detail}</p>
  </main>
  <script>history.replaceState(null,"","/auth/complete")</script>
</body>
</html>`;
}

let mascot: string | undefined;

function mascotDataUri(): string {
  if (mascot === undefined) {
    const file = new URL("../docs/assets/brand/jeco-256.png", import.meta.url);
    mascot = `data:image/png;base64,${readFileSync(file).toString("base64")}`;
  }
  return mascot;
}

function sameState(expected: string, received: string | null): boolean {
  if (received === null) return false;
  const left = Buffer.from(expected);
  const right = Buffer.from(received);
  return left.length === right.length && timingSafeEqual(left, right);
}
