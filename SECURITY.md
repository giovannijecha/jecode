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
sandbox for untrusted projects or commands. Settings include a plaintext
OpenRouter key in the user-scoped configuration; sessions and attachment
copies are also local data. Provider requests send the selected conversation
and supported attachments to OpenRouter and the selected model provider.

Relevant reports include unintended credential exposure, attachment or
history access outside the documented scope, and failures to clean up owned
processes. Expected tool permissions are documented in
[README.md](README.md) and [TOOLS.md](docs/usage/TOOLS.md).
