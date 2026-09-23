# Usage and configuration

## Account and workspace

Start `jecode` in your project directory in an interactive terminal. The first
run displays an OpenAI device sign-in URL and code. Jecode saves the resulting
account credentials and reuses them. Access refresh is serialized across local
instances. If a refresh is interrupted with an uncertain result, sign in again;
the old rotating token is not retried automatically.
Credential checks and mutations use the local lease; model generation releases it
before network I/O so separate instances can work concurrently. Once a request
passes its final local credential check, it may still be sent or finish after
another instance logs out. Local logout removes saved access but cannot revoke
a remote provider session. New requests recheck the saved account and reject
an obsolete client.
An in-progress login or refresh can still require another instance to wait for
the credential lease or retry after its deadline.

`--model MODEL` and `--effort LEVEL` override saved defaults for one new
conversation, including `jecode chat`. With `--model` alone, Jecode omits the
effort field so the account provider chooses its default. `--effort default`
also omits that field; it is distinct from an explicit `none` effort. With
`--effort` alone, Jecode uses the saved default model. These options do not
change saved defaults. Resume always uses the session's saved pair and rejects
model or effort overrides. `--workspace PATH` selects another directory.
`jecode chat` starts a conversation without file or command tools.
It remains associated with its launch directory, including an explicit
`jecode chat --workspace PATH` selection.
The legacy `--account` entry still uses no file tools unless `--workspace` is supplied.

With a workspace, new sessions use the `local` file-access profile: the selected
directory is the base for relative paths, and tools may also address external local
paths. Reads need no extra approval; each change and command still requires it.
`--access workspace` selects bounded access for one new session. `--access local`
explicitly selects the default. With bare `jecode`, the current directory is used.

Run `jecode login` to authenticate without creating an empty conversation.
Esc or Ctrl+C cancels the device flow; a cancelled command exits with status 130.
If an account is already available, the command reports that it is signed in.
Authentication failures report how to retry. Run `jecode logout` to remove
locally saved Jecode account access. Repeating logout leaves the account signed
out. Logout does not revoke remote provider sessions, modify sessions or settings,
or touch credentials from other applications.

Inside a conversation, `/logout` cancels and joins active work before removing
local account access. The transcript and current draft remain visible. `/login`
signs in again in the same conversation. A draft submitted while signed out stays
in the composer and is never sent automatically after login. Esc cancels sign-in;
failures leave the conversation available for another `/login` attempt. Account
codes and local commands are not model messages or saved conversation turns.

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
  "effort": "medium",
  "reduced_motion": false,
  "context_limit_bytes": 524288,
  "file_access": "local"
}
```

Use `/settings` to choose the default model and its reasoning effort, file-access
profile and reduced motion. Model and access defaults apply to new conversations;
existing sessions keep their saved choices. Animation changes apply immediately.
`/model` opens a searchable account model list, then the supported effort choices.
The pair changes only after the final choice is saved, at an idle boundary. Esc
before then keeps the active pair. Neither command starts a new model request.
An `"effort": null` setting selects the provider default. Older settings without
an effort field retain their previous effective medium effort. Catalog metadata
is kept in memory for five minutes and refreshed when needed. An unavailable or
malformed catalog keeps the current choice. If a fresh catalog marks the saved
pair unavailable, the conversation still opens, but Jecode keeps the draft until
you choose a usable pair with `/model`. A missing capability field is treated as
unknown rather than as evidence that an effort is unsupported.
Other settings can be edited while Jecode is closed. The context threshold is between 65,536 and
1,572,864 serialized request bytes. It is an application budget, not a model token
window. `JECODE_REDUCED_MOTION=1` overrides animation; `NO_COLOR=1` disables colors.
`file_access` accepts `local` or `workspace`; omitting it uses `local` for new
sessions. See [tools](TOOLS.md) for the path exclusions and supported formats.

## Sessions

`jecode sessions` shows up to 50 sessions associated with the selected working
directory, most recently active first. `--workspace PATH` selects another
directory for `sessions` and both forms of `resume`; otherwise they use the
current directory. A subdirectory is a separate scope, even inside one Git
repository. Equivalent paths to the same directory share a scope. Each entry
shows its first-message title, working directory, activity age, turn count and
model. Long lines are shortened to fit the terminal; redirected output is plain
text and also includes the full session IDs.

Run `jecode resume` to see the list and choose a session by number, then press
Enter. Esc, Ctrl+C, Ctrl+Q or an empty Enter cancels. The number refers to that
displayed list; no numeric alias is saved. `jecode resume SESSION_ID` opens
a known session directly, but the ID must belong to the selected directory.
An ID from another directory reports its saved location and how to select it;
Jecode never switches directories during resume. Legacy `--sessions` and
`--resume` use the same scope. The saved model, effort and file-access profile are
restored automatically. Older sessions without `effort` retain medium. Reading
or listing a legacy file does not rewrite it; explicit updates preserve unrelated
JSON fields.

Inside a conversation, `/resume` opens a filterable menu of other readable sessions
in that conversation's directory.
Type part of a title or folder, use arrows and Enter to choose, or Esc to return.
The selected ID comes from that captured list. A missing folder or an already
owned session leaves the current conversation open. `/new` starts in the same
directory using saved defaults. Navigation waits for the current operation and
queued guidance to finish. Jecode validates and opens the destination before
releasing the current conversation; the current worker is then joined before
the new conversation accepts input.

Only one instance can own a saved session at a time. Its saved directory must
still be available. No historical tool or pending approval is replayed.
The session retains its file-access profile even if the user default changes.
Older session JSON without `file_access` keeps the original `workspace` profile.
Existing workspace sessions use their saved workspace as the directory association
without changing their files just for listing. New conversation-only sessions save
their launch directory separately and keep zero file tools. Older conversation-only
files without a saved directory have no reliable origin; they remain untouched but
cannot be listed or resumed within a directory scope. File tools using the `local`
profile can still read supported external paths; directory scoping does not narrow
that access.

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
- `/login`: sign in or report that this conversation is already signed in.
- `/logout`: remove local account access and keep this conversation open.
- `/model`: change the model for this conversation.
- `/settings`: change saved defaults and animation.
- `/help`: list local commands.
- `/context`: measured request bytes and the latest available provider token counts.
- `/compact`: summarize earlier turns while keeping the two most recent turns intact.

Startup adds no heading above the conversation. The directory, active
model and effort share one footer row below the composer. Long paths and model
names shorten to fit narrow windows; `/help` shows the directory in full.
Account choices and local feedback stay in the composer; sign-in instructions,
failures and signed-out state appear in the runtime status area above it. The
footer does not gain a permanent authentication indicator.

Type `/` to expand the composer with the command menu, then type to filter.
Up/Down selects, Enter executes and Tab completes the selected command. The menu
shows only command names, between the composer's two lines. Esc closes it and
retains the draft. Help and context reports are printed in terminal scrollback,
leaving the composer ready for the next input; they are not saved as conversation
messages or sent to the model. Model changes update the footer after being saved.
Use Ctrl+Q to save and exit.

Local commands run between turns and do not become model tools. Automatic
compaction runs before a new turn when the configured byte
threshold is exceeded and older turns can be summarized. It uses the selected
model and consumes tokens. A failed or interrupted summary is not automatically
retried; `/compact` requests a new explicit attempt.

Compaction changes only the model-facing projection. Canonical turns and tool
receipts stay saved. A summary can lose detail; retain critical current requirements
in recent messages. If recent content alone exceeds the hard 2 MiB request limit,
start a smaller task or a new session.
