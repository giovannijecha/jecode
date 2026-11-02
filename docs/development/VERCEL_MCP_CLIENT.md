# Vercel MCP client setup

The guided Vercel entry prefills `https://mcp.vercel.com` in the common MCP
form. It uses the same HTTP transport, discovery, OAuth and tool bridge as
other services. It does not use the Vercel CLI or assume that a Vercel REST API
token grants MCP access.

Vercel [requires client review](https://vercel.com/docs/agent-resources/vercel-mcp#connecting-to-vercel-mcp)
for authenticated MCP access. Jecode has not been approved or tested with a
real Vercel account. The Add connection row states this requirement.

## Registration information

| Item | Jecode configuration |
| --- | --- |
| Client name | Jecode |
| Source and homepage | `https://github.com/giovannijecha/jecode` |
| Client type | Native terminal application; public OAuth client |
| Authorization | Authorization code with PKCE S256, protected-resource and issuer discovery |
| Callback | `http://127.0.0.1:<ephemeral-port>/callback` |
| Client registration | Public client ID, Client ID Metadata Document URL, or dynamic registration when offered |
| Identity | Jecode; no credentials or client identity from another approved coding tool |
| Permissions | Teams and resources authorized by the user, independently of local file/command levels |

Submit this information through the client review form linked by Vercel.
No review request has been submitted by this implementation. If approval
requires a public client ID or metadata URL, enter it in the connection's
Advanced > OAuth client ID field. Follow Vercel's registration instructions;
Jecode does not embed a client secret or an invented default ID.

After approval, complete browser sign-in from Settings > MCP and grant the
intended teams. Start verification by listing teams/projects and reading one
deployment. The connection's discovered schemas define the actual tools and
parameters. Do not infer MCP access from a successful CLI/API call; Vercel
[documents their separate authentication](https://vercel.com/docs/agent-resources/vercel-mcp/tools).
