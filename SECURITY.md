# Security

Jecode is experimental software, including its owned TLS implementation. It has
not received an independent security audit.

## Local data

`~/.jecode/v1/` contains ordinary JSON credentials, settings and session records.
There is no credential vault or encryption at rest. Windows creates protected
access rules for the current user and SYSTEM. Linux uses private directories and
files (0700/0600) and rejects unsupported permissions. Administrators, malware
running as your user, backups and disk access remain outside these protections.

Authentication tokens never enter model requests, canonical transcripts or login
error messages. A session can still contain sensitive text that you type, read or
approve a command to produce. Keep this directory out of version control and do
not share it as a diagnostic bundle.

`--logout` removes Jecode's locally saved account access. It does not revoke the
account at the provider or erase other processes' memory. An already sent request
can finish; later requests from another Jecode instance check saved account state.

## Tools

Workspace tools validate arguments, paths and bounds. File changes require a
preview and a matching approval; stale files are rejected and existing originals
have a recovery copy. Historical tool calls are never executed on resume.

An approved shell runs with your user permissions. Its starting directory is not
a sandbox: it can access other files or the network. Review the command before
approving it. Cancellation stops owned processes and waits for cleanup, but cannot
undo effects that have already happened.

A crash between an effect and its saved receipt leaves an unknown outcome. Inspect
the workspace before repeating that action. Jecode does not infer success or
silently retry ambiguous model requests.

## Reporting

Report reproducible security concerns through GitHub's private vulnerability
reporting when available. Avoid public issues containing credentials, personal
sessions or exploit details affecting other users. Include the affected version,
platform, minimal synthetic reproduction and observed impact.

See the [TLS profile](docs/TLS.md) for certificate and protocol limitations.
