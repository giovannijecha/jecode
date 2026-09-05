// Inspect the production OAuth result page without signing in or storing data.

import { createServer } from "node:http";
import { oauthResultPage } from "../../src/oauth-result-page.ts";

const [state = "success", ...extra] = process.argv.slice(2);
if ((state !== "success" && state !== "failure") || extra.length > 0) {
  throw new Error("usage: npm run web:preview -- [success|failure]");
}

const page = oauthResultPage(state === "success");
const server = createServer((_request, response) => {
  response.writeHead(200, {
    "content-type": "text/html; charset=utf-8",
    "cache-control": "no-store",
  }).end(page);
});
server.listen(0, "127.0.0.1", () => {
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("preview address is unavailable");
  process.stdout.write(`OAuth ${state} preview: http://127.0.0.1:${address.port}\n`);
});
