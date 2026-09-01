# Security Policy

## Supported versions

Jecode is currently pre-1.0. Security fixes are applied to the latest tagged
release and the `main` branch.

| Version | Supported |
|---|---|
| 0.3.x | Yes |
| 0.2.x and older | No |

## Report a vulnerability

Do not open a public issue for a suspected vulnerability. Use GitHub's private
vulnerability reporting from the repository's **Security** tab.

Include the affected version, operating system, Node.js version, reproduction
steps, expected impact, and any proposed mitigation. Do not include API keys,
private source code, saved credentials, or conversation transcripts.

Relevant security boundaries include terminal control sequences, credential
storage or transport, workspace path confinement, command approval, process
cancellation, and unbounded remote or filesystem input.
