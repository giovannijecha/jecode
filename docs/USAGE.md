# Usage and configuration

## Account and workspace

Start `jecode --account --workspace PATH` in an interactive terminal. The first
run displays an OpenAI device sign-in URL and code. Jecode saves the resulting
account credentials and reuses them. Access refresh is serialized across local
instances. If a refresh is interrupted with an uncertain result, sign in again;
the old rotating token is not retried automatically.

`--model gpt-5.6-luna` or `--model gpt-5.6-terra` overrides the configured model
for a new session. Both currently use medium effort. Omitting `--workspace` starts
a conversation with no file access or command tools.

Run `jecode --logout` to remove locally saved account access. This does not modify
sessions, settings or credentials from other applications.

## User directory

All persistent application state uses the user home directory (`USERPROFILE` on
Windows, `HOME` on Linux):

```text
~/.jecode/v1/
  credentials.json
  settings.json
  sessions/
    s-...json
```

Small lock files coordinate instances and remain after shutdown. Temporary JSON
files are atomically replaced; no database or separate service is required.
Existing files outside this versioned directory are neither imported nor modified.
Keep user data on a local filesystem that supports private permissions and file
locking. Under WSL use the Linux home directory rather than a DrvFS directory
without Unix metadata permissions.

`settings.json` is created with these defaults:

```json
{
  "version": 1,
  "model": "gpt-5.6-luna",
  "reduced_motion": false,
  "context_limit_bytes": 524288
}
```

Edit settings while Jecode is closed. The context threshold is between 65,536 and
1,572,864 serialized request bytes. It is an application budget, not a model token
window. `JECODE_REDUCED_MOTION=1` overrides animation; `NO_COLOR=1` disables colors.

## Sessions

`jecode --sessions` lists up to 50 sessions with their IDs and first-message titles.
`jecode --resume SESSION_ID` opens a session with its saved model and workspace.
Only one instance can own a saved session at a time. The workspace must still be
available at its saved path. No historical tool or pending approval is replayed.

History is saved before model work, around tool effects, and when a turn ends.
Normal cancellation retains partial output. An abrupt process or machine failure
may lose the in-flight streamed portion since the last checkpoint; an effect with
no durable receipt remains explicitly uncertain. Corrupt or unsupported JSON is
reported rather than overwritten with an empty session.

The resumed transcript shows messages, tool summaries and outcomes. Full provider
items, tool arguments, results and recorded turn metrics remain in the session JSON.
The current limits are 256 turns, 16 MiB per session snapshot and 8 KiB per message.

## Input and context

Enter sends the draft. During generation it queues guidance; up to eight messages
can wait for delivery. Guidance is inserted at a boundary between model/tool
steps. If a turn has just finished, it starts the next turn. It never grants an
approval. On cancellation, unsent queued messages are shown as not sent. A queue
waiting for acceptance is in memory; do not rely on it surviving a forced exit.

Esc interrupts. Ctrl+Q exits after active work is cancelled and joined. Terminal
scrollback remains available. Bracketed paste does not submit text automatically.

- `/help`: list local commands.
- `/context`: measured request bytes and the latest available provider token counts.
- `/compact`: summarize earlier turns while keeping the two most recent turns intact.
- `/quit`: exit.

Tab completes an unambiguous command prefix. Local commands do not become model
tools. Automatic compaction runs before a new turn when the configured byte
threshold is exceeded and older turns can be summarized. It uses the selected
model and consumes tokens. A failed or interrupted summary is not automatically
retried; `/compact` requests a new explicit attempt.

Compaction changes only the model-facing projection. Canonical turns and tool
receipts stay saved. A summary can lose detail; retain critical current requirements
in recent messages. If recent content alone exceeds the hard 2 MiB request limit,
start a smaller task or a new session.
