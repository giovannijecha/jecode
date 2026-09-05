// The browser handoff back to the terminal after account sign-in.

import { readFileSync } from "node:fs";

export function oauthResultPage(success: boolean): string {
  const title = success ? "You're signed in" : "Sign-in failed";
  const detail = success
    ? "Return to your terminal. You can close this tab."
    : "Return to your terminal to try again.";
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="dark">
  <title>${title} · Jecode</title>
  <style>
    :root {
      color-scheme: dark;
      --background: #1c2026;
      --text: #ebeff4;
      --muted: #9ca9b7;
      --accent: #669bd2;
      --error: #e87070;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100vh;
      min-height: 100svh;
      display: grid;
      place-items: center;
      background: var(--background);
      color: var(--text);
      font-family: "Segoe UI", system-ui, sans-serif;
    }
    main { width: min(28rem, 100%); padding: 2rem; }
    .identity { display: flex; align-items: center; gap: .75rem; margin-bottom: 2rem; }
    .identity img { display: block; width: 3rem; height: 3rem; }
    .identity span {
      color: var(--accent);
      font: 600 1.25rem/1 ui-monospace, "Cascadia Mono", Consolas, monospace;
      letter-spacing: -.04em;
    }
    h1 { margin: 0; font-size: 1.75rem; font-weight: 600; line-height: 1.25; letter-spacing: -.025em; }
    p { margin: .75rem 0 0; color: var(--muted); font-size: 1rem; line-height: 1.6; }
    .failure h1 { color: var(--error); }
  </style>
</head>
<body>
  <main${success ? "" : ' class="failure"'}>
    <div class="identity">
      <img src="${mascotDataUri()}" width="48" height="48" alt="">
      <span>jecode</span>
    </div>
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
    const file = new URL("../assets/jeco-256.png", import.meta.url);
    mascot = `data:image/png;base64,${readFileSync(file).toString("base64")}`;
  }
  return mascot;
}
