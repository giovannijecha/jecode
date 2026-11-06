# MCP connections

Manage global HTTP MCP connections in **Settings > MCP**. `/mcp` inspects only
configured connections and their tools. Multiple connections to the same
service use distinct names. They apply to every conversation and collaborator.
Plain mode has no MCP settings; use the TUI or
[edit the file directly](#edit-the-file-directly).

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
uses its registered App for device sign-in. Other services and custom servers
use Jecode's public Client ID Metadata Document when supported, then dynamic
registration when available; **Advanced > Custom OAuth client** accepts a
pre-registered client ID or metadata URL instead.
Entering `Name: value` adds or replaces that header; `Name:` removes it.
Esc discards unsaved edits. Saving a changed endpoint replaces the old server
identity and authentication; it never forwards the old token to a new URL.

| Service | HTTP endpoint | Authentication |
| --- | --- | --- |
| GitHub | `https://api.githubcopilot.com/mcp/` | Token or OAuth with the registered Jecode App and a browser/device code. |
| Supabase | `https://mcp.supabase.com/mcp` | Token or OAuth discovered from the server. |
| Cloudflare | `https://mcp.cloudflare.com/mcp` | API token or OAuth discovered from the server. |
| Vercel | `https://mcp.vercel.com/` | OAuth with Client ID Metadata Document; you approve access in Vercel. |

The client and UI have isolated synthetic tests. Explicit Windows account checks
on 2026-11-03 completed OAuth, token refresh and tool discovery with all four
services. Cloudflare returned four entry-point tools; access to account resources
was not exercised. These checks do not run remote mutations. Vercel accepts
[unreviewed clients with public metadata](https://vercel.com/changelog/vercel-mcp-now-supports-client-id-metadata-documents);
you still approve each connection. See the [GitHub App setup](../development/GITHUB_MCP_APP.md)
for browser sign-in, which is optional for token connections.
Jecode ships its public GitHub client ID; users do not register an App or
configure an identifier. They authorize Jecode and choose the repositories
they grant it access to. A public server with no authentication reads
**Not needed**; a token-only server still asks for the explicit Token choice.
The [Vercel client setup](../development/VERCEL_MCP_CLIENT.md) records the
public metadata URL and publication requirement. Vercel CLI/API tokens are not
assumed to authorize the MCP endpoint; access follows its OAuth grants.

## Edit the file directly

`~/.jecode/mcp.json` is a JSON object with a `connections` array:

```json
{
  "connections": [
    {
      "id": "docs",
      "name": "Docs",
      "endpoint": "https://example.com/mcp",
      "method": "none"
    }
  ]
}
```

`id` uses letters, digits and dashes. `method` is `none` or `token`, which also needs a
`token` field holding the bearer token. Optional fields are `enabled` (default `true`),
`timeout_secs` (1–600, default 60) and `headers`. Browser and device sign-in records are
written by the TUI's sign-in flow.

## Global configuration and permissions

Connections are stored in the user-scoped `~/.jecode/mcp.json`, separate from
model configuration. The file contains plaintext tokens and has owner-only
operating-system permissions. Credentials stay out of model results, terminal
output and conversation journals: tokens, OAuth tokens and credential-like
header values (names containing key, token, secret, password, auth, session or
signature, cookies, or values of 20 characters or more) are redacted. `mcp list`
shows the model only each endpoint's origin, so credentials embedded in a URL
path or query stay local. An unreadable `mcp.json` is reported at startup;
Jecode continues without MCP and the `mcp` tool returns the error.

Suspending or removing a connection blocks future calls globally. It does not
cancel an operation already accepted by the service. Remote permissions follow
the enabled connection and the service's grants, independently of local Read,
Workspace and Full. Server annotations do not grant permission or prove that a
tool is read-only. Local checkpoints cannot undo remote changes.

[OAuth](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)
uses protected-resource and authorization-server metadata, an OAuth client,
PKCE S256 and a loopback callback on an ephemeral port. It supports a configured
client ID or client metadata URL, Jecode's public metadata when advertised, and
dynamic registration as a fallback. A dynamically registered client serves
only the sign-in that registered it and that session's refreshes; the next
sign-in registers again. Registration can assign `none`, `client_secret_basic`
or `client_secret_post`; omitted server authentication methods default to
`client_secret_basic`. Per-registration client secrets are stored and
redacted with the session credentials. Refresh remains bound to the original
endpoint, issuer and client. Query options such as
[Supabase's `project_ref`](https://supabase.com/docs/guides/ai-tools/mcp) remain
part of the connection identity even when the OAuth resource URI omits the
query. Reconnect from Settings when sign-in has expired or access was revoked.
After the token exchange succeeds the browser shows a black `jecode authenticated`
page; the connection itself is saved later, after its own check, and reports in
Jecode. A failed sign-in shows `jecode not authenticated`.
The authorization server must advertise S256 support in its metadata; otherwise
browser sign-in is unavailable. A bearer token remains an alternative.

When the authorization server advertises `offline_access` and the sign-in
request names scopes, Jecode adds it so the server can issue a refresh token.
Without any requested scopes, Jecode omits `scope` and the server applies its
defaults. Issuance remains the server's choice. A tool-specific
`403 insufficient_scope` retains the required scope for the next sign-in in
Settings, together with scopes previously granted by the same authorization
server. Changing the endpoint discards a retained scope. The failed remote
operation is not replayed automatically after authorization.
Sign-in reloads current credentials and requested permissions before opening
the browser. Saving a new grant preserves later permission requests for the
same endpoint and rejects a saved connection removed or changed meanwhile.

For the stock GitHub endpoint, Jecode requests the default toolsets and Actions.
Custom toolset/tool headers and custom endpoint paths retain their selection.
The GitHub App and installation must grant the corresponding permissions; each
installation owner approves permission increases.

## Inspect tools and results

`/mcp` shows configured, suspended, expired and unreachable connections, with
available and configured counts kept separate. Select a connection to filter
its tools, then select a tool for its description and input schema. Unavailable
connections can open their Settings entry; Esc returns to inspection.

The model uses `mcp list`, `mcp tools` and `mcp call`; remote schemas are loaded
on demand rather than appended to every model request. Negotiated sessions
are reused until configuration changes or an error invalidates the client; a
session the server expired (HTTP 404) is renewed and the request sent once
more, because the server did not process it. Tool cards and Work chapters
follow the existing design; Ctrl+O expands the full arguments and result.

Results stay within the inline tool-output limit. Image, audio and blob data
are replaced by a placeholder with their type and size, and an oversized result
keeps a text preview with `truncated: true`. In both cases the full result is
saved and returned as `result_file`, which `read` opens.
Discovery pages follow the same limit: if descriptions, server instructions or
input schemas make a page too large, it returns a preview and `result_file`.
Read that file for the exact definitions; schemas are never cut into partial
objects. The limit includes the response's identity and outcome fields.

A failed tool, a JSON-RPC error or an HTTP 4xx rejection returns an error with
the server's reason. If a dispatched call loses its response, times out, gets
an HTTP 5xx or is interrupted, its outcome is **unknown**: the service may
already have acted. Jecode never automatically replays it, including after a
crash. Inspect the remote state before choosing to repeat it.

## Protocol scope

The owned HTTP client supports MCP 2026-07-28 and negotiated Streamable HTTP
2025-11-25 / 2025-06-18 / 2025-03-26, JSON and SSE responses, paginated
catalogs, session headers and parameter headers. Text and structured content
remain in the JSON receipt. Embedded image/audio blocks are not converted into
model attachments. SSE streams the server closes before the response are not
resumed with `Last-Event-ID`; such calls report an unknown outcome.

This first implementation exposes tools. Stdio executables, the older standalone
SSE transport, resources, prompts, sampling, tasks and interactive elicitation
are outside its current scope. A server requesting additional input receives
an actionable unsupported result rather than a false completed receipt.
