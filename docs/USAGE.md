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

On Windows, commands use system Windows PowerShell 5.1 unless you explicitly
select a PowerShell 7 executable. To select one, add its absolute path to
`~/.jecode/v1/settings.json` while Jecode is closed. For example:

```json
{
  "version": 1,
  "model": "gpt-5.6-luna",
  "effort": "medium",
  "reduced_motion": false,
  "context_limit_bytes": 524288,
  "file_access": "local",
  "windows_powershell_executable": "C:\\Program Files\\PowerShell\\7\\pwsh.exe"
}
```

Jecode starts the configured executable when starting or resuming a session
to identify its PowerShell version and runs one bounded bracketed-directory
capability check through the command runner against an isolated temporary fixture.
The selected version, executable and check result appear in command previews and
model instructions. A missing, invalid or unavailable selection reports an error;
Jecode does not fall back to 5.1. Remove
the field or set it to `null` to use the system default. Changes to the setting
take effect for the next started or resumed session. PowerShell 7 is supplied by
the user and is not bundled with Jecode. PowerShell 7.6.6 is the CI-tested version;
other 7.x versions are assessed at session start rather than accepted or rejected
by version number. A failed semantic check or an inconclusive check (such as a
temporary-fixture or probe launch failure) prevents commands from starting in a
directory containing `[` or `]`, while commands from ordinary directories remain
available. Start a new session to retry an inconclusive check.

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
The current limits are 256 turns and 16 MiB per session snapshot. A submitted
user message is limited to 8 KiB. Reaching a snapshot or turn limit stops new
work with an error; canonical history is not discarded to make room.

## Input and context

Enter sends the draft. Ctrl+O inserts a newline; Ctrl+J also works when the
terminal delivers it separately from Enter. Windows console key records also
distinguish Shift+Enter and Ctrl+Enter. Some terminals intercept modified keys or
send the same code for two shortcuts; Ctrl+O is the newline fallback. Bracketed
paste keeps line breaks, indentation, tabs and Unicode. CRLF and lone CR become LF.
Pasted slash-prefixed text remains a prompt, not a local command. An insertion or
paste that would exceed 8 KiB is rejected in full with a message inside the
composer; the prior draft stays editable. A rejected submission also keeps the
draft.

Left/Right move by display-safe text units. Ctrl+Left/Right move by words when
delivered; VT terminals can also use Alt+B/F. Ctrl+Backspace/Delete delete a word
where distinguishable, and Ctrl+W deletes the previous word. The Windows console
path distinguishes DEL from BS when the terminal sends those different codes;
if both keys send the same code, use Ctrl+W or VT Alt+Backspace.
Home/End move to the start/end of the current logical line; Ctrl+A/E do the same.
Ctrl+Home/End move to the start/end of the whole draft where delivered. Up/Down
move through visual rows of a multiline or wrapped draft. On a single visual row, Up recalls a user
prompt and Down returns toward the current draft. With no menu open, Page Up/Down
browse prompt history even with a multiline draft; Ctrl+P/N are alternatives if the
terminal takes the Page keys. The unsent draft and its cursor are
restored after the newest entry. Up to 64 recent user prompts are recalled from
the current session's canonical turns on resume. Local commands, login codes,
intraturn guidance and provider output are excluded. Guidance that starts a new
canonical turn is available for recall. Recalled prompts remain
editable and never send automatically.

During generation Enter queues guidance; up to eight messages can wait for
delivery. Short previews appear inside the composer. Guidance is inserted at a
boundary between model/tool steps. If a turn has just finished, it starts the
next turn. It never grants an approval.

Alt+Up withdraws the newest message that is still pending and puts it in the
editor. Edit it and press Enter to submit it through the normal path. A message
already claimed for delivery cannot be withdrawn; the current draft stays put
and the composer reports that nothing is pending. Recovered slash-prefixed text
remains literal input. The previous draft, including its cursor, returns after
the recovered edit is successfully submitted. If submission fails, both remain
available. Alt+Up cannot replace an edit already being recovered. Alt+Down
explicitly discards that recovered edit and restores the previous draft; it
does not requeue or send anything. Esc still cancels active work without clearing
the editor. Logout keeps both drafts available while the conversation remains
open.
If you browse prompt history from a recovered edit, use Ctrl+N (or Page Down) to
return to that edit before pressing Enter. Submitting a recalled entry is
blocked while the withdrawn text is hidden, so neither draft is lost.

Pending guidance and the saved draft exist only in this running process. On
cancellation, logout or delivery failure, guidance that was never sent appears
in scrollback marked "Queued message was not sent" for manual copying; it is
never resent automatically. Ctrl+Q joins active work and exits, but unsent
composer and queue text is not stored for a later launch.

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
- `/compact`: summarize completed context, including completed steps of the active turn.

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

Local commands run between turns and do not become model tools. Before every
generation request, Jecode measures the serialized model request and compacts
completed turns or completed active-turn steps when the configured byte threshold
is exceeded. Each summary uses a bounded slice and the selected model and effort;
it consumes tokens. A failed or interrupted summary is not automatically retried
within that task; `/compact` requests a new explicit attempt.

Compaction checkpoints only the model-facing projection. Canonical turns, tool
arguments and exact receipts stay saved and are not replayed on resume. A summary
can lose detail; restate critical requirements if needed. The account request
encoder accepts at most 2 MiB of JSON and 4,096 input items. A single indivisible
completed step or current input that cannot fit a bounded summary request may stop
with a context error. These are byte and item bounds, not a claimed model token
capacity. Provider token counts are reported separately when available. The
16 MiB session snapshot and 256-turn limits remain separate storage boundaries;
larger canonical storage needs a separate format and migration workstream.
