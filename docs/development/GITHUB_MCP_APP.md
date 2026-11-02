# GitHub MCP App registration

GitHub token authentication works immediately: add the [remote GitHub MCP endpoint](https://github.com/github/github-mcp-server/blob/main/docs/remote-server.md), `https://api.githubcopilot.com/mcp/`, in Settings > MCP, choose Token, and paste a GitHub token whose permissions are appropriate for the intended tools. Keep the token in the local MCP connection store. Jecode does not create a GitHub App or infer permissions from a connection name.

Browser-based device sign-in requires an App client ID. The maintainer has not registered this App yet. To enable it later:

1. [Register a GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app) under the intended owner. Set a homepage URL and choose the minimum repository and organization permissions needed by the intended MCP tools. Keep user authorization token expiration enabled.
2. Enable Device Flow in the App settings. GitHub ignores the callback URL for device flow. Installation and user authorization still determine the repositories and operations a resulting token can access.
3. Copy the **Client ID**, not the App ID or client secret, into the connection's OAuth client ID field or set `JECODE_GITHUB_CLIENT_ID` when starting Jecode. Do not commit it as a hardcoded default. The device flow uses `offline_access` so expiring tokens include a refresh token.
4. Connect from Settings > MCP. Jecode shows GitHub's verification URL and user code, then polls at GitHub's required interval. A refresh of a device-generated token needs the client ID and refresh token, not a client secret.

GitHub documents the [device flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app#using-the-device-flow-to-generate-a-user-access-token) and [refresh requirements](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens#refreshing-a-user-access-token-with-a-refresh-token). The App's permissions and the user's access bound the token; the OAuth scope does not grant additional GitHub App permissions.

No GitHub registration or credentials are part of the repository build. Do not test this flow against a personal account in automated tests; use a synthetic loopback server.
