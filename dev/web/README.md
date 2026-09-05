# Browser pages

Preview the production OAuth result page with inert success or failure states:

```powershell
npm run web:preview
npm run web:preview -- failure
```

Open the printed loopback URL. Stop the server with Ctrl+C. The preview does
not authenticate, contact providers, or read account data. It renders
[`src/oauth-result-page.ts`](../../src/oauth-result-page.ts) directly.

The page has one job: confirm the sign-in outcome and send the user back to
the terminal. Keep Jeco small, one outcome heading, and one next-step sentence.
Use the Slate colours, system fonts, and a static layout without decorative
effects. Success and failure share the same structure. Check both states at
desktop and narrow widths before changing this page.
