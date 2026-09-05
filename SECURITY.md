# Security

Security fixes are shipped in the latest Jecode release. Update first and
confirm that the issue still occurs before reporting it.

## Report a vulnerability

Do not open a public issue. Send a
[private vulnerability report](https://github.com/giovannijecha/jecode/security/advisories/new)
instead.

Describe what happens, why it matters, the smallest way to reproduce it, and
the relevant environment. Include sanitized logs or screenshots only when they
help. Never include API keys, tokens, private source code, workspace content, or
conversation transcripts.

## Understand the boundary

Jecode keeps its built-in filesystem tools inside the selected workspace, asks
before dangerous actions by default, and redacts credentials it recognizes.

Jecode is not an operating-system sandbox. A shell command you approve can
access the same files and accounts as your user. Review commands and remembered
approval scopes carefully. `/permissions` lets you change session tool policies
and revoke remembered approvals; `/new` restores their defaults.

See the [safety model](docs/USAGE.md#understand-the-safety-model) for the detailed
technical boundaries.
