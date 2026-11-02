# MCP connections

Manage global HTTP MCP connections in **Settings > MCP**. `/mcp` inspects only
configured connections and their tools. Multiple connections to the same
service use distinct names. They apply to every conversation and collaborator.

## Connect a server

1. Open `/settings`, choose **MCP**, then **Add connection**.
2. Choose GitHub, Supabase, Cloudflare, Vercel or **Server URL**. Guided entries only
   prefill the common form; every service uses the same protocol client.
3. Open **Sign-in** and explicitly choose **Token** or **OAuth** from the
   methods offered by the server. Token opens the masked field. OAuth opens
   browser authorization directly and shows a one-time code only when needed.
4. Save. Jecode checks the connection and loads the tools before saving it.

The form includes Name, Endpoint and Sign-in. Advanced contains the request
timeout (1–600 seconds, default 60) and masked custom headers. Only GitHub
supplies its client identity internally. Other services and custom servers use
dynamic registration when the server offers it; **Advanced > Custom OAuth
client** accepts a pre-registered client ID or metadata URL instead.
Entering `Name: value` adds or replaces that header; `Name:` removes it.
Esc discards unsaved edits. Saving a changed endpoint replaces the old server
identity and authentication; it never forwards the old token to a new URL.

| Service | HTTP endpoint | Authentication |
| --- | --- | --- |
| GitHub | `https://api.githubcopilot.com/mcp/` | Token or OAuth with the registered Jecode App and a browser/device code. |
| Supabase | `https://mcp.supabase.com/mcp` | Token or OAuth discovered from the server. |
| Cloudflare | `https://mcp.cloudflare.com/mcp` | API token or OAuth discovered from the server. |
| Vercel | `https://mcp.vercel.com` | OAuth; authenticated access requires Vercel's client approval. |

These are connection targets, not claims of live account verification. The
client and UI are tested with isolated synthetic servers. Vercel controls its
[client approval](https://vercel.com/docs/agent-resources/vercel-mcp); Jecode
does not bypass it. See the [GitHub App setup](../development/GITHUB_MCP_APP.md)
for browser sign-in, which is optional for token connections.
Jecode ships its public GitHub client ID; users do not register an App or
configure an identifier. They authorize Jecode and choose the repositories
they grant it access to. A public server with no authentication reads
**Not needed**; a token-only server still asks for the explicit Token choice.
The [Vercel client setup](../development/VERCEL_MCP_CLIENT.md) records the
approval requirement and registration details. Vercel CLI/API tokens are not
assumed to authorize the MCP endpoint; access follows its OAuth grants.

## Global configuration and permissions

Connections are stored in the user-scoped `~/.jecode/mcp.json`, separate from
model configuration. The file contains plaintext tokens and has owner-only
operating-system permissions. Credentials stay out of model results, terminal
output and conversation journals.

Suspending or removing a connection blocks future calls globally. It does not
cancel an operation already accepted by the service. Remote permissions follow
the enabled connection and the service's grants, independently of local Read,
Workspace and Full. Server annotations do not grant permission or prove that a
tool is read-only. Local checkpoints cannot undo remote changes.

[OAuth](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)
uses protected-resource and authorization-server metadata, a public client,
PKCE S256 and a loopback callback on an ephemeral port. It supports a configured
client ID or client metadata URL, with dynamic registration when offered. A
dynamically registered client serves only the sign-in that registered it and
that session's refreshes; the next sign-in registers again. Refresh remains
bound to the original endpoint, issuer and client. Query options such as
[Supabase's `project_ref`](https://supabase.com/docs/guides/ai-tools/mcp) remain
part of the connection identity even when the OAuth resource URI omits the
query. Reconnect from Settings when sign-in has expired or access was revoked.

## Inspect tools and results

`/mcp` shows configured, suspended, expired and unreachable connections, with
available and configured counts kept separate. Select a connection to filter
its tools, then select a tool for its description and input schema. Unavailable
connections can open their Settings entry; Esc returns to inspection.

The model uses `mcp list`, `mcp tools` and `mcp call`; remote schemas are loaded
on demand rather than appended to every model request. Negotiated sessions
are reused until configuration changes or an error invalidates the client.
Tool cards and Work chapters follow the existing design; Ctrl+O expands the
full arguments and result.

A failed tool returns an error. If a dispatched call loses its response or is
interrupted, its outcome is **unknown**: the service may already have acted.
Jecode never automatically replays it, including after a crash. Inspect the
remote state before choosing to repeat it.

## Protocol scope

The owned HTTP client supports MCP 2026-07-28 and negotiated Streamable HTTP
2025-11-25 / 2025-06-18, JSON and SSE responses, paginated catalogs, session
headers and parameter headers. Text, structured content and other result
blocks remain in the JSON receipt. Embedded image/audio blocks are not
converted into model attachments.

This first implementation exposes tools. Stdio executables, the older standalone
SSE transport, resources, prompts, sampling, tasks and interactive elicitation
are outside its current scope. A server requesting additional input receives
an actionable unsupported result rather than a false completed receipt.
