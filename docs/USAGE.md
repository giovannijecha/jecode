# Usage and configuration

## Account and workspace

Start `jecode` in your project directory in an interactive terminal. The first
run displays an OpenAI device sign-in URL and code. Jecode saves the resulting
account credentials and reuses them. Access refresh is serialized across local
instances. If a refresh is interrupted with an uncertain result, sign in again;
the old rotating token is not retried automatically.

`--model gpt-5.6-luna` or `--model gpt-5.6-terra` overrides the configured model
for a new session. Both currently use medium effort. `--workspace PATH` selects
another directory. `jecode chat` starts a conversation without file or command tools.
The legacy `--account` entry still uses no file tools unless `--workspace` is supplied.

With a workspace, new sessions use the `local` file-access profile: the selected
directory is the base for relative paths, and tools may also address external local
paths. Reads need no extra approval; each change and command still requires it.
`--access workspace` selects bounded access for one new session. `--access local`
explicitly selects the default. With bare `jecode`, the current directory is used.

Run `jecode logout` to remove locally saved account access. This does not modify
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
  "context_limit_bytes": 524288,
  "file_access": "local"
}
```

Use `/settings` to change the default model, file-access profile and reduced motion.
Model and access defaults apply to new conversations; existing sessions keep their
saved access. Animation changes apply immediately. `/model` changes the current
conversation's model after the turn finishes and saves that choice for resume.
Other settings can be edited while Jecode is closed. The context threshold is between 65,536 and
1,572,864 serialized request bytes. It is an application budget, not a model token
window. `JECODE_REDUCED_MOTION=1` overrides animation; `NO_COLOR=1` disables colors.
`file_access` accepts `local` or `workspace`; omitting it uses `local` for new
sessions. See [tools](TOOLS.md) for the path exclusions and supported formats.

## Sessions

`jecode sessions` shows up to 50 sessions, most recently active first. Each entry
shows its first-message title, working directory, activity age, turn count and
model. Long lines are shortened to fit the terminal; redirected output is plain
text and also includes the full session IDs.

Run `jecode resume` to see the list and choose a session by number, then press
Enter. Esc, Ctrl+C, Ctrl+Q or an empty Enter cancels. The number refers to that
displayed list; no numeric alias is saved. `jecode resume SESSION_ID` opens
a known session directly. Legacy `--sessions` and `--resume` also work;
the saved model, workspace and access profile are restored automatically.

Inside a conversation, `/resume` opens a filterable menu of other readable sessions.
Type part of a title or folder, use arrows and Enter to choose, or Esc to return.
The selected ID comes from that captured list. A missing folder or an already
owned session leaves the current conversation open. `/new` starts in the same
directory using saved defaults. Navigation waits for the current operation and
queued guidance to finish. The previous worker is joined before another starts.

Only one instance can own a saved session at a time. The workspace must still be
available at its saved path. No historical tool or pending approval is replayed.
The session retains its file-access profile even if the user default changes.
Older session JSON without `file_access` keeps the original `workspace` profile.

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

- `/new`: start a new conversation in the same directory.
- `/resume`: find and reopen another saved conversation.
- `/model`: change the model for this conversation.
- `/settings`: change saved defaults and animation.
- `/help`: list local commands.
- `/context`: measured request bytes and the latest available provider token counts.
- `/compact`: summarize earlier turns while keeping the two most recent turns intact.
- `/quit`: exit.

Type `/` to open the menu, then type to filter. Up/Down selects, Enter executes
and Tab completes the selected command. Esc closes a menu before cancelling work.
Local commands do not become model
tools. Automatic compaction runs before a new turn when the configured byte
threshold is exceeded and older turns can be summarized. It uses the selected
model and consumes tokens. A failed or interrupted summary is not automatically
retried; `/compact` requests a new explicit attempt.

Compaction changes only the model-facing projection. Canonical turns and tool
receipts stay saved. A summary can lose detail; retain critical current requirements
in recent messages. If recent content alone exceeds the hard 2 MiB request limit,
start a smaller task or a new session.
