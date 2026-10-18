# Personal configuration

Use `/settings` to update saved defaults or the OpenRouter key. `jecode setup`
opens guided configuration outside an active conversation.

## Stored settings

Settings live outside your project in `~/.jecode/config.json`. On Windows this is
`%USERPROFILE%\.jecode\config.json`. The format is:

```json
{
  "openrouter": {
    "api_key": "your-key",
    "model": "provider/model-id",
    "effort": "default"
  },
  "context": {
    "auto_compact_percent": 80
  }
}
```

Credentials are deliberately stored in plain text. Jecode does not load `.env`
files or print the key. Conversations autosave under `~/.jecode/sessions/`, with
the configured key masked. Configuration writes are atomic; unknown fields are
retained for future settings. Corrupt, incomplete, oversized
or read-only files are left unchanged and reported. Configuration is limited to
64 KiB. New Unix configuration directories/files use modes 0700/0600; Windows
uses the permissions inherited from the user's directory.

## Automatic compaction threshold

`context.auto_compact_percent` sets where automatic context reduction starts, as
a share of the effective context window: the model's catalog `context_length`,
lowered by any ceiling learned from provider context rejections. It accepts
whole numbers from 1 to 99 and defaults to 80; files written before this
setting load with 80. A value outside that range makes the file invalid, like
other malformed settings, and the file is left unchanged.

Change it in `/settings` under **Auto-compact at**. Enter a whole percentage,
optionally followed by `%`; an invalid entry is rejected and the previous value
stays active and saved. A successful save applies from the next model request of
the current conversation and to every new or resumed conversation; it is a user
preference, not saved per conversation. If the file cannot be written, the
previous value stays active. `jecode setup`, model and effort changes and key
replacement keep the saved value.

The input budget, this percentage of the effective window, both triggers
reduction and limits its result. The output allowance is calculated separately
from the space left in the effective window, so a higher threshold leaves less
room for the model's reply before reduction starts. A threshold too small for the system instructions, original
user requests and a useful summary stops the turn with a resource error naming
the budget; history and the previous context are preserved. See
[LONG_WORK.md](LONG_WORK.md#active-context-and-original-history).

## Precedence and environment

The saved configuration takes precedence over legacy environment variables.
`--model` overrides the model for one run without changing saved defaults. A
changed model starts with its default effort; the saved pair applies otherwise.
Older configuration files without `effort` keep working and use `default`.
If no file exists, `OPENROUTER_API_KEY` and `OPENROUTER_MODEL` can still configure
a run without saving credentials to `config.json`; sessions still autosave.
`--model` may supply the environment-only model.
`jecode setup` always manages the personal file.

## Advanced overrides

- `JECODE_HOME`: absolute configuration directory, useful for isolated development.
- `JECODE_BASH`: path to the installed Bash executable.
- `INSTALL.ps1 -InstallRoot ABSOLUTE_PATH -NoPath`: install elsewhere without
  changing PATH, useful for verification.

## Conversation overrides

Model and effort changes keep the conversation and do not overwrite saved
defaults. Settings apply to future `/new` conversations and launches; changing
the key also updates authentication for the current conversation, and
Auto-compact at also applies to the current conversation. Across a
model boundary, portable content and tool calls/results remain in context.
Older provider-specific reasoning fields stay in the export rather than being
sent to the new model. Within the same model they are retained for tool follow-up.

See [SESSIONS.md](SESSIONS.md) for persisted model/effort state and explicit resume.
