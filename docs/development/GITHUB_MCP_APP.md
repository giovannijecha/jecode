# Jecode GitHub MCP App

Jecode uses one maintainer-owned public GitHub App for every user's GitHub
OAuth login. Users choose **Token** or **OAuth** in Settings > MCP. OAuth opens
GitHub in the browser and displays a device code when required. Users do not
register an App, enter a client ID or handle a client secret.

## Registered identity

| Field | Value |
| --- | --- |
| App | [Jecode GitHub MCP](https://github.com/apps/jecode-github-mcp) |
| Owner | `giovannijecha` |
| App ID | `5580672` |
| Public client ID | `Iv23liTdak2Sf8VpPtLw` |
| Homepage | `https://github.com/giovannijecha/jecode` |
| Availability | Any GitHub user or organization |
| Authorization | Device Flow enabled; expiring user tokens; authorization during installation disabled |
| Repository permissions | Contents, Issues and Pull requests: read/write; Metadata: read |
| Webhooks | Inactive |

The maintainer registered this App on 2026-11-02 and its public identity and
permissions were verified through GitHub's API. The public client ID is bundled
in `src/mcp/auth/github.rs`, so source, CI and release builds share the same
identity without a release-environment variable. Maintainer overrides through
`JECODE_GITHUB_CLIENT_ID` remain available; end users need no setup variable.

## User authorization and installation

An installation selects which repositories Jecode may access. Every user or
organization makes its own selection. The user's access, the App's permissions
and the installation's repository selection together bound its tokens.
Organizations may require administrator approval. A public App is available
to other accounts; it is not automatically installed everywhere.

GitHub's [device flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app#using-the-device-flow-to-generate-a-user-access-token)
uses the public client ID, a verification page and a one-time user code. Neither
initial device authorization nor [device-token refresh](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens)
needs a client secret. Jecode stores the resulting user tokens in the private
MCP connection store; no token is part of the build.

## Maintainer controls

The App owner manages Device Flow, installability and permission changes in
GitHub's App settings. [Permission increases require approval](https://docs.github.com/en/apps/maintaining-github-apps/modifying-a-github-app-registration#changing-the-permissions-of-a-github-app)
before existing installations gain them. The current permissions support code,
branch, issue and pull-request work; organization administration, Actions and
secrets are outside the initial grant.

GitHub's registration UI required a private key before installation. The owner
keeps that key privately. Jecode's user/device OAuth flow does not use it: never
put an App private key or client secret in source, builds, logs or release
artifacts. [GitHub documents private-key handling](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/managing-private-keys-for-github-apps).

[GITHUB_MCP_APP_MANIFEST.json](GITHUB_MCP_APP_MANIFEST.json) records the initial
registration fields for maintenance. It contains no credentials. GitHub's
manifest creation flow still requires owner browser consent; `GH_TOKEN` alone
does not provide a direct create-App request. Device Flow is configured in
GitHub's registration UI.

Automated tests use isolated loopback fixtures. The App registration and
installation are complete; a real end-user OAuth login through Jecode remains
part of manual account validation.