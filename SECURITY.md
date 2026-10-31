# Security

Security fixes target the current `main` branch. Jecode is under active
development; older revisions have no separate support schedule.

## Report a vulnerability

Use [GitHub's private vulnerability reporting](https://github.com/giovannijecha/jecode/security/advisories/new).
Include the affected revision and operating system, the boundary that was
crossed and a minimal reproduction using synthetic files and credentials. Do not
put security details or real secrets in a public issue or pull request.

The maintainer will assess the report and coordinate a fix and disclosure. There
is no guaranteed response time.

Relevant reports include unintended credential exposure, access to files,
history or attachments outside the documented scope, permission checks that can
be bypassed, and owned processes that are not cleaned up.

## Data sent to providers

Each model request sends the conversation to the selected provider: your
messages, the agent's answers, tool results (including file contents, command
output, fetched web pages and browser snapshots the agent has read), project
instructions and supported attachments.

- **OpenRouter** forwards requests to the model provider it selects.
- **OpenAI** receives requests directly through the Responses API, using your
  ChatGPT account.
- **Anthropic** receives requests directly through the Messages API, using your
  Claude account through a compatibility route; see
  [Providers](docs/usage/PROVIDERS.md#claude-account).

Known credentials are masked in conversation text, tool output, saved
conversations and text exports. Attached images and PDFs are sent as file
content; Jecode does not inspect or redact secrets inside them.

## Credentials

- The **OpenRouter API key** is stored in plain text in
  `~/.jecode/config.json`.
- **ChatGPT and Claude tokens** are stored in plain JSON account records under
  `~/.jecode/accounts/`, readable only by your user: owner-only permissions on
  Unix, an ACL restricted to the current user on Windows. Access tokens
  authenticate requests; refresh tokens are sent only to the issuing provider
  to renew them, and for ChatGPT to revoke them at sign-out. The verified
  ChatGPT ID token stays in the local record.
- Commands the agent runs do not inherit provider credentials from Jecode's
  environment.

## Local data

Jecode keeps local data under `~/.jecode/`, including sensitive copies of:

- saved conversations, complete command output and attachment copies;
- checkpoints, which hold unredacted copies of project files, including files
  Git ignores such as `.env`;
- temporary files, web responses and browser data;
- named browser profiles under `~/.jecode/browser/profiles/`, which can keep
  cookies and signed-in sessions.

## Boundaries

[Permission levels](docs/usage/PERMISSIONS.md) govern what the agent's tools may
do. On supported Windows hosts, Workspace commands and their descendants run
under an OS-enforced filesystem policy; on Linux and macOS, commands require
Full and run unconfined. Full commands run with your normal user permissions.
Permissions do not isolate the whole Jecode process, remote services or
everything a browser does.

The `web` tool uses Jecode's native HTTP support with operating-system TLS and
certificate verification. The `browser` tool controls an installed browser
through a local DevTools connection. Both can reach any destination your
machine can reach. Account sign-in uses your system browser and a callback
listener bound to `127.0.0.1`.
