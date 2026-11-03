# Language servers

Jecode can query language servers you install and explicitly enable. Its owned
LSP client handles the protocol; servers provide language analysis. Jecode does
not bundle, download, update or automatically enable a server found on PATH.
Without a configured server, the other tools remain available.

## Configure a server

Open **Settings > LSP > Add server**, then choose Rust / rust-analyzer,
C/C++ / clangd or Custom server. Guided entries prefill the same form. Select
an executable path, or a command name resolved by the server's environment.
For a script-based server, select its runtime executable and put the script
path and its arguments in Advanced.
Languages pairs language IDs with file extensions, for example
`rust: .rs` or `c: .c .h; cpp: .cc .cpp .hpp`. A custom server uses the same
client and configuration; adding a language requires no language-specific code.

Advanced contains arguments, masked environment variables, root markers or
a fixed root, initialization options, settings and the request timeout.
Arguments are an argv list; quoted spaces remain part of one argument.
Initialization options and settings are JSON supplied to the server.

New entries visibly default On. **Save** stores the entry and its enablement
without starting a process. **Check** explicitly starts a temporary instance,
initializes it, reads advertised capabilities and stops it. A Check leaves no
conversation instance; it can also be requested for an entry that is Off.
Failed checks and Esc preserve saved settings. Editing fields creates a draft;
save is explicit.

Server entries apply globally. Instances belong to one conversation and root,
start lazily on a matching model request, and are reused there. **Turn off**
blocks new dispatch, cancels waiting work and stops owned instances. Changes
to analysis settings or removal stop affected instances. Turning On and
**Allow restart** do not start a server by themselves. Conversation changes
and shutdown stop the old conversation's instances.

## Execution scope

Like MCP enablement, LSP enablement is independent of Read / Workspace / Full.
The selected executable runs as your user with its configured environment and
toolchain behavior, including any subprocesses it starts. The selected root
is analysis context; it does not confine filesystem or network access. The
model cannot select arbitrary executables, enable servers or apply edits
requested by a server through this first LSP tool. Local file and command
tools keep their existing [permission policy](PERMISSIONS.md).

Prefer an explicit root when a language workspace encloses the Jecode working
folder. Otherwise configured markers are searched from the requested file
up to the working folder; Git is not invoked and discovery does not climb
outside that boundary. An explicit wider root is a user choice.

## Inspect and query

`/lsp` is read-only: it shows configured entries and observed instances of
this conversation. Opening it or Settings starts and checks nothing. Ready
means initialized; indexing or diagnostics may still be pending.

The model has one `lsp` tool with these actions:

| Action | Input and result |
| --- | --- |
| `status` | Optional stable `server` ID; configuration and observed state, without starting a server. |
| `definition` | `path`, one-based `line` and Unicode-scalar `column`; normalized locations and ranges. |
| `references` | The same position, plus optional `include_declaration`; normalized locations. |
| `diagnostics` | `path`; errors and warnings for that document, with freshness and coverage. |

An optional `server` ID resolves multiple matching enabled entries. Large
responses have an inline preview and `result_file`; the model can read the
retained response. Inline truncation is separate from protocol completeness.
No results, unsupported capabilities, an Off entry and unavailable analysis
have distinct outcomes.

Disk is authoritative. The client opens managed documents and reconciles
changes, including external edits, before queries. Bounded portable polling
also handles active documents and registered workspace watches. A scan budget
gap is reported rather than treated as complete synchronization.
The same bounded observer follows configured file associations and root markers.
When a dependency changes, it resynchronizes managed snapshots with increasing
protocol versions and invalidates prior diagnostics; source files are not
rewritten. Push and pull reports are combined, so an empty pull cannot erase
a current compiler diagnostic delivered by push.

Diagnostics can be current, pending, stale, unverified or unavailable. An
empty current report means no reported errors or warnings for its stated
document only. A versionless push report after an edit does not establish
freshness. Initialization can advertise pull diagnostics; absence of that
advertisement does not rule out later push reports. Errors found are successful
analysis results, never an automatic completion gate.

Timeout and cancellation stop the caller's request; a healthy initialized
server remains available. Cancellation is cooperative and late obsolete
responses are ignored. Cancelling the first startup before dispatch stops
its unused instance. Crashes fail pending requests; repeated failures suspend
automatic restarts until you choose Allow restart.

## Configuration file

Global entries are stored in `~/.jecode/lsp.json`, with a separate lock file.
Plain mode uses the same model tool; configure entries in the TUI or edit
this file. `/lsp` also provides read-only inspection in plain chat.
Environment values are retained locally in plaintext and masked in the TUI.
Credential-like values (names containing key, token, secret, password, auth,
session or signature, or values of 20 characters or more) are redacted from
tool output; short ordinary values such as `1` or `info` stay readable.

```json
{
  "servers": [
    {
      "id": "rust",
      "name": "Rust / rust-analyzer",
      "enabled": true,
      "executable": "rust-analyzer",
      "arguments": [],
      "environment": {},
      "languages": ["rust"],
      "extensions": {"rs": "rust"},
      "root_markers": ["Cargo.toml", "rust-project.json"],
      "root_override": null,
      "initialization_options": null,
      "settings": {},
      "request_timeout_ms": 10000
    }
  ]
}
```

The implementation uses the established semantic features and capability
negotiation of the official [Language Server Protocol](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/).
Formatting, rename, code actions, workspace edits, remote transports and
server installation are outside this first integration.
