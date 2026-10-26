# Security

Security fixes target the current `main` branch. The project is under active
development; older revisions do not have a separate support schedule.

## Reporting a vulnerability

Use [GitHub's private vulnerability reporting](https://github.com/giovannijecha/jecode/security/advisories/new).
Include the affected revision and operating system, the boundary crossed,
and a minimal reproduction with synthetic files and credentials. Do not put
security details or real secrets in a public issue or pull request.

The maintainer will assess the report and coordinate a fix and disclosure.
There is no guaranteed response time.

## Boundaries

Jecode runs local tools with the invoking user's permissions. It is not a
sandbox for untrusted projects or commands. The OpenRouter API key is stored in
plaintext in the user-scoped configuration, while ChatGPT tokens are stored in
separate protected account records; sessions, attachment copies
and workspace checkpoint archives are also local data. Checkpoint archives hold
unredacted project file contents, including Git-ignored secrets such as `.env`.
Model requests send the selected conversation and supported attachments to the
active backend. OpenRouter forwards them to its selected model provider; OpenAI
receives them directly through the Responses API using the selected ChatGPT plan.
The ChatGPT access token authenticates OpenAI requests; its refresh token is
sent to OpenAI for renewal or revocation. The verified sign-in ID token stays
in the local account record; it is not sent for inference, refresh or returning
sign-in. Credential records use
owner-only Unix permissions or a Windows ACL restricted to the current user.
Refreshed tokens are masked by existing output and persistence consumers.
The `web` tool makes direct HTTP(S) requests from the user's machine using local
curl. The `browser` tool controls a locally installed Chromium-based browser
through a loopback DevTools connection. Fetched content, browser snapshots,
screenshots and logs may enter the conversation and provider request when the
agent uses them. Temporary browser profiles and downloaded content live in the
session area; named persistent profiles under `~/.jecode/browser/profiles/`
can retain cookies and signed-in state. These tools have the same local user
permissions as Jecode and can access destinations reachable from that machine.

Relevant reports include unintended credential exposure, attachment or
history access outside the documented scope, and failures to clean up owned
processes. Expected tool permissions are documented in
[README.md](README.md) and [TOOLS.md](docs/usage/TOOLS.md).
