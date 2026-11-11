# Jecode GitHub MCP App

Jecode uses one maintainer-owned public GitHub App for every user's GitHub
OAuth login. Users choose **Token** or **OAuth** in Settings > MCP. OAuth opens
GitHub's device page in the browser and displays the one-time code to enter
there. Users do not register an App, enter a client ID or handle a client
secret.

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
| Repository permissions | Actions, Contents, Issues, Pull requests and Workflows: read/write; Checks, Commit statuses and Metadata: read |
| Webhooks | Inactive |

The maintainer registered this App on 2026-11-02 and its public identity and
permissions were verified through GitHub's API and updated on 2026-11-03. The public client ID is bundled
in `src/mcp/auth/github.rs`, so source, CI and release builds share the same
identity without a release-environment variable. Maintainer overrides through
`JECODE_GITHUB_CLIENT_ID` remain available; end users need no setup variable.
Sign-in does not copy the bundled ID into the connection, so a later bundled or
maintainer ID applies to the next sign-in. Refresh keeps the client ID that
issued the session. GitHub App permissions, not OAuth scopes, bound the tokens.

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
branch, issue and pull-request work, workflow-file changes and Actions operations.
Checks and commit statuses are readable. Organization administration and secrets
are not requested.

The stock remote endpoint enables the default toolsets plus `actions` through
`X-MCP-Toolsets`. Explicit tool filters and custom endpoint paths keep their
configured selection. GitHub filters the available tools and operations by the
user's granted permissions; enabling a toolset does not grant repository access.

GitHub's registration UI required a private key before installation. The owner
keeps that key privately. Jecode's user/device OAuth flow does not use it: never
put an App private key or client secret in source, builds, logs or release
artifacts. [GitHub documents private-key handling](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/managing-private-keys-for-github-apps).

[GITHUB_MCP_APP_MANIFEST.json](GITHUB_MCP_APP_MANIFEST.json) records the public
registration fields for maintenance. It contains no credentials. GitHub's
manifest creation flow still requires owner browser consent; `GH_TOKEN` alone
does not provide a direct create-App request. Device Flow is configured in
GitHub's registration UI.

Automated tests use isolated loopback fixtures. On 2026-11-03, an explicit live
Windows check completed device authorization, discovered 48 tools including
Actions, refreshed the session and discovered tools again. This does not execute
workflow writes or change the repository selection of other installations.
