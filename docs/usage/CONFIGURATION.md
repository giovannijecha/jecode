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
the key also updates authentication for the current conversation. Across a
model boundary, portable content and tool calls/results remain in context.
Older provider-specific reasoning fields stay in the export rather than being
sent to the new model. Within the same model they are retained for tool follow-up.

See [SESSIONS.md](SESSIONS.md) for persisted model/effort state and explicit resume.
