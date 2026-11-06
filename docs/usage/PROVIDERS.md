# Providers

Jecode connects to three model providers. API keys are the recommended route;
subscription sign-in remains a separate choice:

| Provider | How you connect | What you pay with |
| --- | --- | --- |
| OpenRouter | An OpenRouter API key | OpenRouter credits, at the selected model's rates. |
| OpenAI API | An OpenAI API key | OpenAI API billing, separate from a ChatGPT plan. |
| Anthropic API | An Anthropic API key | Claude API billing, separate from a Claude plan. |
| OpenAI account | Sign in with your ChatGPT account | Your ChatGPT plan's usage. |
| Anthropic account | Sign in with your Claude account (compatibility route; read [the caveat](#claude-account)) | Your account's usage settings. |

You can configure API keys and subscription accounts together, and select
their models independently. Jecode does not retry a failed API request through
subscription credentials, or a failed subscription request through an API key.
You can configure several providers and switch between them per conversation.

Whatever you choose, Jecode sends the conversation to that provider, including
project files and command output the agent has read, and supported attachments.
With OpenRouter, the request goes on to the model provider OpenRouter selects.
See [Security](../../SECURITY.md#data-sent-to-providers).

## Connect in the TUI

Start `jecode` in a project. Without a configured provider the TUI still opens,
shows **No configured provider** and points to `/settings`.

1. Type `/settings` and press Enter, then choose **Providers**.
2. Choose **API keys** → **OpenRouter**, **OpenAI API** or **Anthropic API**
   for a key, or **Credentials** → **OpenAI** or **Anthropic** → **Add account**
   for an account.
3. Finish the connection (see the sections below).
4. Choose a model when the model selector opens. Your first model choice saves
   the configuration and becomes the default for new conversations.

Until you choose that first model, nothing is written to
`~/.jecode/config.json`; cancelling the selector discards an API key you
entered. Account sign-in records are saved as soon as sign-in succeeds.

## Connect from the command line

`jecode setup` runs the same configuration as a guided prompt. It lists API-key
connections before subscription accounts, connects the one you pick, checks
access without making a model request, lets you search for a model and saves it
as the default:

```text
jecode setup
```

It ends with `Saved. Model: <model>`. In a model search, type `=` followed by an
ID, such as `=provider/model-id`, to enter an exact model ID.

`jecode --plain` and a `jecode "PROMPT"` run in an interactive terminal also
start setup when the saved provider has no credentials. Piped tasks never
prompt; they fail with an instruction to run `jecode setup`.

## OpenRouter API key

Create a key at <https://openrouter.ai/settings/keys>. In the TUI, choose
**API keys** → **OpenRouter** and paste the key; it shows as dots while you
type. In `jecode setup`, the key is entered hidden and checked with OpenRouter
before you choose a model. Pressing Enter on an empty key keeps the saved one.

The key is stored in plain text in `~/.jecode/config.json`. Keep that file
private. OpenRouter-specific endpoint choices are described in
[Configuration](CONFIGURATION.md#openrouter-endpoint-routing).

## OpenAI API key

Create a key at <https://platform.openai.com/api-keys>, then choose
**API keys** → **OpenAI API**. Key entry, checking, model selection and saving
follow the same steps as OpenRouter. Jecode sends requests directly to
OpenAI's Responses API; usage is billed to the API project, separately from
any ChatGPT subscription.

The API model list does not report context windows or reasoning levels.
The selector omits known model families for embeddings, images, moderation,
audio, transcription and realtime endpoints; unknown IDs remain available.
Jecode leaves context unknown and provides the documented reasoning levels for
known API models, including `gpt-6.1-sol`. Other model IDs keep `default` in the
menu; `/effort NAME` can still request a level explicitly. API and subscription
capabilities remain separate. The API can reject models or settings that your
project does not support; see the [OpenAI adapter reference](../development/OPENAI.md#direct-api-key).

## Anthropic API key

Create a workspace-scoped key at <https://platform.claude.com/settings/keys>,
then choose **API keys** → **Anthropic API**. The same masked input and model
selection flow stores it in `~/.jecode/config.json`. Requests go directly to
the Claude Messages API using Jecode's identity; they do not use the Claude
Code compatibility profile. Usage is billed to the API workspace, separately
from a Claude subscription.

Keys that require an explicit workspace header are not currently configurable;
use a workspace-scoped key. Provider rejections remain visible and do not cause
a switch to subscription credentials.

## ChatGPT account

Choose **Credentials** → **OpenAI** → **Add account**. Jecode opens your browser
on OpenAI's sign-in page and waits for the redirect on a local `127.0.0.1` port.
Complete the sign-in within five minutes. In `jecode setup`, press Enter while
it waits to cancel and return to the provider list. When the tokens
are saved, the browser shows a black page reading `jecode authenticated` and
`close this tab`; if the sign-in fails it reads `jecode not authenticated` and
`close this tab and try again`.

This uses OpenAI's documented
[Sign in with ChatGPT flow for open-source apps](https://developers.openai.com/siwc/token-sharing-open-source/sign-in).
Requests consume your ChatGPT plan's usage. When the plan's limit is reached,
Jecode stops and points to
[ChatGPT Settings → Usage](https://chatgpt.com/settings/usage); it never falls
back to paid API billing. **Manage usage** in the account menu opens the same
page.

The model list, context window and supported reasoning levels come from your
account's catalog. Implementation details are in
[OpenAI backend](../development/OPENAI.md).

## Claude account

Choose **Credentials** → **Anthropic** → **Add account**. Jecode opens your
browser on claude.ai's authorization page and waits for the redirect on a local
port. Complete the sign-in within five minutes. The browser
ends on the same black `jecode authenticated` page once the tokens are saved, or
`jecode not authenticated` if the sign-in failed.

Read this before using it:

- This is a compatibility route, not an approved integration. Anthropic's
  [Agent SDK documentation](https://code.claude.com/docs/en/agent-sdk/overview)
  states that, unless previously approved, Anthropic does not allow third-party
  developers to offer claude.ai login or rate limits for their products. Jecode
  has no such approval.
- The authorization page names the client as Claude Code, and model requests
  present Claude Code's client identity. Jecode does not install or run Claude
  Code or read its credentials.
- Anthropic can restrict or reject this route at any time, independently of
  Jecode releases.

Jecode shows this compatibility status before sign-in in both setup and the
TUI and recommends **API keys** → **Anthropic API** instead. The warning does
not claim that an account will be banned or predict how a request is billed.

Requests use your account's usage settings. Jecode does not enable extra usage,
change spending limits or fall back to API billing. The full compatibility
profile and verification limits are in
[Anthropic backend](../development/ANTHROPIC.md).

**Manage usage** in the Claude account menu opens
<https://claude.ai/settings/usage> to view the account's limits and usage
settings. It does not change those settings from Jecode.

## Manage accounts

Open `/settings` → **Providers** → **Credentials** and choose a provider to see
its saved accounts. Choose an account to **Use account**, **Manage usage** or
**Sign out**. Usage pages belong to the provider and may use the account already
signed in in your browser.

- **Sign out** deletes that account's local tokens. For ChatGPT, Jecode also
  asks OpenAI to revoke them; for Claude, the remote authorization is not
  revoked.
- Account records are stored separately from `config.json`, under
  `~/.jecode/accounts/openai/` and `~/.jecode/accounts/anthropic/`. They are
  plain JSON files readable only by your user (owner-only permissions on Unix,
  a current-user ACL on Windows).
- Tokens refresh automatically. Jecode masks known tokens and keys in tool
  output, saved conversations and exports.

## Switch providers and models

With more than one provider configured, `/model` first separates **API keys**
and **Credentials**, using the same categories as provider settings. Choose a
category, a configured provider, then the model and effort. The change applies
to the current conversation only. `/settings` → **Default model** uses the same
categories for the default of new conversations. See
[Configuration](CONFIGURATION.md#models-and-effort).

If the default provider loses its credentials, the TUI uses another configured
provider for the conversation, restores that provider's last saved model when
there is one, and shows a warning with the reason. A `--model` override is
skipped in that case, because model IDs differ between providers. Your saved
defaults do not change. Single prompts and piped tasks fail instead.

## Troubleshooting

| Message or symptom | What to do |
| --- | --- |
| `No configured provider` | Connect one in `/settings` → **Providers**, or run `jecode setup`. |
| `Signed in, but ChatGPT plan access was not granted` (or `Claude plan access`) | The account signed in, but the provider did not grant plan use. Check the account's plan, then sign in again. |
| A rejected API key | Enter a valid key under **API keys** for the same provider; check its API project/workspace access and billing. |
| The ChatGPT plan's usage limit is reached | Inference stops; check [ChatGPT Settings → Usage](https://chatgpt.com/settings/usage). Jecode does not retry this error. |
| `Default provider unavailable · … · Using X for this conversation` | The default provider has no usable credentials. Reconnect it, or keep working with the fallback. |
| `Jecode is not configured. Run jecode setup in a terminal, then retry your task.` | A pipe or redirected run found no configuration. Run `jecode setup` interactively first. |
| Sign-in never completes | The attempt expires after five minutes. Start it again; make sure the browser can reach `localhost` and `127.0.0.1`. |

## Environment variables

Without a configuration file, `OPENROUTER_API_KEY` and `OPENROUTER_MODEL` can
configure a run without saving anything. Set `OPENROUTER_API_KEY` and either
`OPENROUTER_MODEL` or `--model`; when both name a model, `OPENROUTER_MODEL`
wins. Once `config.json` exists, these variables are ignored.

```sh
OPENROUTER_API_KEY=YOUR_OPENROUTER_KEY OPENROUTER_MODEL=provider/model-id jecode "Summarize this project"
```

Replace `YOUR_OPENROUTER_KEY` and `provider/model-id` with your key and a model
ID from the OpenRouter catalog. This form is for POSIX shells; in PowerShell,
set `$env:OPENROUTER_API_KEY` and `$env:OPENROUTER_MODEL` first.
