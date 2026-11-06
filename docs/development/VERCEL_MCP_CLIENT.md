# Vercel MCP client setup

The guided Vercel entry prefills `https://mcp.vercel.com/` in the common MCP
form. Vercel's protected-resource metadata announces that exact resource URI.
The connection uses the common HTTP transport, discovery, OAuth and tool bridge;
it does not use the Vercel CLI or treat a Vercel REST API token as an MCP grant.

Vercel [accepts clients with a Client ID Metadata Document (CIMD)](https://vercel.com/docs/agent-resources/vercel-mcp#client-identification-with-cimd)
without prior client review. Users still approve each connection. Review is
optional for inclusion in Vercel's supported-clients list.

## Public client identity

Jecode's native public-client document is
[`.well-known/mcp-oauth-client.json`](../../.well-known/mcp-oauth-client.json)
in the repository. Its `client_id` is
`https://giovannijecha.github.io/jecode/mcp-oauth-client.json`. Publication
maps that source document to the exact client ID URL on a static Pages branch;
the URL must return the JSON document directly over HTTPS before a Vercel
sign-in can use it. The document carries Jecode's name and source, the
`http://127.0.0.1/callback` loopback redirect, authorization-code and refresh
grants, and `token_endpoint_auth_method: none`. Jecode listens on an ephemeral
loopback port and uses PKCE S256. No client secret or logo is bundled.

The authorization server advertises CIMD support, so Jecode selects this URL
by default. **Advanced > Custom OAuth client** can override it with another
client ID or metadata URL. If an authorization server does not support CIMD,
the generic client can use its dynamic registration endpoint instead.

The document is served as `application/json` by GitHub Pages from the dedicated
`gh-pages` branch. That branch contains the published document and `.nojekyll`;
application source and local working files are not part of the site. Publish
metadata updates from the canonical source above and verify the exact public
document before shipping a changed identity.

On 2026-11-03, an explicit live Windows check completed the native loopback
authorization flow, discovered 241 tools, refreshed the token and discovered
tools again. Jecode adds `offline_access` when the authorization server advertises
it and the sign-in request names other scopes; Vercel requires that scope to issue refresh tokens. Although its
metadata lists `web_message.opener`, the native query callback was verified.
The check performs no deployment or project mutation.
