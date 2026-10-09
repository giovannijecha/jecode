# Composer and conversation controls

The shared Windows, Linux and macOS interface opens fullscreen in the terminal's
alternate buffer. One conversation column scrolls above a composer fixed at the
bottom. Page Up/Down and the mouse wheel browse Jecode's conversation; Up/Down
move within the draft outside panels and tool inspection. Alt+Home goes to the
beginning and Alt+End resumes following new output, including while a panel is
open. Reading older content pauses following until the viewport returns to the
bottom. A centered blue Back to bottom badge appears above the composer while
reading older content. Esc returns to the latest output during normal reading;
panels, draft edits, history recall and tool inspection receive Esc first, so
use Alt+End in those contexts. Mouse selection is omitted; `/copy` retains
original text.

Resize keeps the reading anchor, logical draft/caret, selector query/selection
and queue. Source reflow begins after 75 ms without another dimension change;
input and generation continue between layout passes. The main shell screen and
native scrollback are preserved. Exit restores them before printing a short
summary and, for a saved conversation, its resume command. `--plain` remains the
line-oriented alternative. See [RESIZE.md](RESIZE.md).

## Conversation spacing

Visible conversation blocks have one neutral blank separator between them and
no separator before the first block. An entire connected tool tree is one block:
headers, previews, diffs, capture notices and successive calls are adjacent.
Explicit blank output lines keep their indentation within the preview limit;
one terminating output newline does not add an extra row.
UI feedback and local command output occupy no transcript rows, including after
resume, and add no separators. Their audit records remain in saved sessions and
JSON exports. Feedback and requested information use the temporary lower area.

Sent requests have one blank row above and below their text, both on the user
panel background. They have no `›` marker. A request with N visual text rows
occupies N + 2 rows. These colored padding rows are additional to the neutral
separator: user/assistant transitions have two blank rows between written
content, and two adjacent user panels have three.

Assistant blocks have no vertical padding. Headings, paragraphs, lists, quotes
and wrapped continuation rows have no automatic margins. Explicit initial and
internal blank rows are retained, including repeated blank rows. Horizontal
rules occupy one row. A complete fenced code panel has one language row, N
visual code rows and one empty bottom row, all on the code background. Without
a language, its top row is empty too. Blank code rows retain the code background;
blank Markdown rows outside the fence stay neutral.

Assistant text is cleaned for display and trimmed at the end before rendering;
raw conversation/export content is unchanged. Empty or whitespace-only assistant
updates occupy no rows and leave a tool tree open. Streaming updates grow the
same block and finalization preserves its layout.

Normally one neutral blank row separates visible conversation content from the
entire lower area: queue, notices, activity and composer/selector. Those lower
rows have no extra separators between them. The gap belongs to the lower area
and contains the reading/reflow hint when needed. With less than two rows available
above the lower area, it is omitted to keep the input and some context reachable.

## Draft and footer

The idle composer has four rows: full-width top border, input, bottom border,
and a gray directory/model/effort footer. Both borders and the `›` marker are blue
at rest and gray during generation. Ordinary draft text inherits the terminal
foreground. The gray placeholder is `Ask anything…`.

The software caret is a static light block over the character at the insertion
position. At the end it highlights a space; when empty it highlights the
placeholder's `A`. The terminal's native cursor stays hidden until exit.
Palette values are in [THEME.md](THEME.md).

Newlines and soft wrapping count as visual rows. The composer shows up to five
consecutive rows around the caret. Longer drafts scroll this window without
pinning the first row or inserting hidden-line notices into the text. A muted
visible-range indicator sits in the top border only when part of the draft is
outside the window:

```text
────────────────────────────── 3–7 / 7 ─
› line 3
  line 4
  line 5
  line 6
  line 7█
────────────────────────────────────────
C:\project               model · default
```

The indicator counts visual rows, including wrapping, and updates with the
caret and terminal size. Short windows show fewer consecutive rows; narrow
windows omit the indicator before reducing space for editing. Hidden text stays
in the draft. Directory text is shortened with `…` to leave
room for the model and effort on one footer row. Very small viewports retain the
input/caret and show fewer supporting rows as necessary.

## Commands and selectors

Typing a leading `/` replaces the bordered composer with a shaded command panel.
Command input and suggestions share the same surface as menu titles, search,
options and controls. The background matches sent user messages, with two-cell
side margins and one colored padding row above/below when space permits.
The selected row uses a lighter background and blue `›`; command text is light,
matching letters are blue and descriptions are gray. No composer borders or
directory/model/effort footer appear inside this panel.

Without whitespace, the panel shows up to six command suggestions:
`/new`, `/resume`, `/drafts`, `/model`, `/effort`, `/settings`,
`/help`, `/copy`, `/export`, `/tmp`.
Matching is case-insensitive; contiguous matches rank before scattered letters.

Up/Down moves selection. Tab completes the command. Enter executes the selected
command when idle; while working it queues the draft as written, except `/copy`
and `/drafts`, which open immediately. Esc dismisses the panel without clearing
text or moving the draft caret, including during work; editing can reopen it.
Adding whitespace closes suggestions while retaining command input in the panel.
Unknown commands produce local
errors and, for nearby misspellings, a correction suggestion. They never reach
the model. `/clear`, `/setup`, `/exit` and `/quit` remain compatibility commands.

Model, effort, settings, session, draft and copy selectors use the same shaded panel.
Titles and optional visible ranges share one row, with muted metadata aligned
right, as in `Model` and `1–8 / 376`. Search or masked key input uses a separate
row below. Their search
text/caret is separate from the draft. Eight options or fewer use numbered menus:
arrows/Enter or a number selects directly. Larger lists use search, up to eight
visible options. Filtered lists whose results all fit show a compact match count
in the title; unfiltered lists whose options all fit show no count. Hidden-option
notices consume no additional rows. Command suggestions and `/help` or `/tmp`
use the same visible-range format; multiline command input takes priority in
the command title. Very narrow headings omit metadata to keep the title readable.
Esc cancels
without applying a partial selection or losing the draft. Within `/settings`,
it returns from a child picker to the settings menu; Esc at that menu closes it.
Small viewports drop padding and reduce visible options, retaining the caret,
selected row and controls before supporting rows. Pending deletion prioritizes
the marked row and confirmation controls. Feedback stays above the panel on the
terminal surface.
Narrow panels use compact keyboard hints; `↵` means Enter and `^D` means Ctrl+D.

- `/model [ID]` selects a tool-capable model, then an effort. Both apply together
  after the effort choice. `/effort [NAME]` also accepts a direct effort.
- Efforts are limited to the catalog's declared capabilities; `default` leaves
  reasoning settings to the provider. It does not claim an unknown model uses
  `high`. Mandatory reasoning models do not offer `none`.
- `/model` and `/effort` affect this conversation only. Content and tool pairs
  remain in context; old provider-specific reasoning stays in the export across
  model changes. Sessions also retain the compatibility boundary for explicit
  resume. Automatic context compaction preserves the original conversation.
- `/settings` changes the saved default model/effort or OpenRouter key. Defaults
  take effect on `/new` and future launches; the current footer stays unchanged.
  Key input is masked, validated in the background and saved only on success.
  Changes and failed actions return to the refreshed settings menu with feedback;
  cancelled child pickers return silently. Use Close or Esc to leave it. Standalone `/model`
  and `/effort` selections still return directly to the conversation.
- `/resume [ID]` restores a saved session in the current folder. Its list includes
  the current conversation after its first sent request; empty launches and
  unsent input do not create a row. Ctrl+D marks the selected row for deletion, Enter
  confirms and Esc cancels the mark in the same list. Search or navigation also
  cancels the mark. Deletion keeps the list open, including when it becomes empty
  or the current conversation is deleted. Current deletion starts a fresh context
  with unsent composer, queue and paused drafts kept; Esc returns to that composer.
  File removal runs in the background; Ctrl+Q waits for it to finish.
- `/drafts` or Alt+Up opens pending drafts immediately, even during a turn. The
  main composer and caret are preserved. Queued messages appear first, followed
  by paused drafts; identical text remains in separate rows. Enter opens the
  selected item in the composer with `Editing draft N/M`. Enter saves the edit
  in its original slot and returns to the list. Esc or Ctrl+C cancels the edit
  and returns to the list; Esc there closes it. Ctrl+D marks only the selected
  row for discard. Enter confirms, Esc cancels, and number keys never confirm.
  Ctrl+S explicitly sends or queues a paused draft from the list or its editor
  and restores the original main draft.
- `/tmp` shows the session's temporary path, counts, size and retention policy
  in a temporary information panel.
  `/tmp clean` clears that area only when ready; it queues during generation.
  Drafts, project files, saved output and conversation history are retained.
- Local commands never add UI feedback to the transcript. Plain mode retains
  its command output.

## Feedback and information

Notices share the panels' `#282E39` background and two-cell indent, with no dot.
Information and successful results use gray text; warnings use gold and errors
use red. Long text wraps within the available width. Brief results, previous
session hints, copy results and turn summaries disappear after five seconds or
when editing starts. Errors remain until the next user action. Copy feedback
has an independent slot so it does not replace an unrelated session error.
Closing or cancelling a selector adds no confirmation.

Loading appears once in the operation's title, and ends with the operation.
Connection recovery and maintenance use one temporary progress row in place of
the ordinary activity row. Notifications never become conversation rows, including
during resize, and old UI messages are not replayed on resume.

`/help` and `/tmp` information replace the composer with the same shaded panel.
They do not expire automatically. Up/Down scroll visual rows; Page Up/Down move
one page and Home/End reach the ends. Keys and values align when space permits
and stack in narrow windows. Esc, Ctrl+C or Enter closes the panel without
sending the draft. Typing or editing returns to the draft and applies that input
at its preserved caret. The queue waits while an information panel is open.

## Activity and queue

An activity row appears above the draft or menu content during a turn. Within
command/menu panels it shares their background:

```text
••••  Thinking  ·  3.2s                            Esc stops
───────────────────────────────────────────────────────────
› next draft█
───────────────────────────────────────────────────────────
C:\project                                     model · high
```

`Waiting for the model` means a model request is pending, `Thinking` requires a
real reasoning event, `Running tools` follows tool execution and `Writing the
response` follows streamed text. The timer spans the entire
turn, including waits and tools: tenths of a second below a minute, then `1m 04s`.
Four identical round bullets start at the leftmost conversation column. A blue
brightness wave advances every 120 ms without moving or changing their glyphs.
The label and input caret stay static. Tool headers show textual status without
animation. Temporary connection/provider failures retry automatically; recovery
uses a temporary progress row. See [LONG_WORK.md](../usage/LONG_WORK.md#connection-recovery).

The draft remains editable while working. Enter queues at most eight messages,
clearing the input only on success. Queued previews appear oldest first above
notices/activity; the Drafts list replaces these previews while open. Multiline
messages show their first line with `…`; short
viewports keep the next message visible and put the visible range in the first
preview's right-aligned `queued` label. Overflow adds no summary row. Narrow
previews prioritize the next message over metadata.

After a successful turn, messages run in FIFO order. Local commands run locally;
prompts start new turns. An open selector, information panel or draft edit pauses
queue consumption. Closing the panel resumes ordinary queued messages. `/drafts`
also opens during work: edit an individual pending message with Enter, or use
Ctrl+S to send or queue a paused draft explicitly. Alt+Down does not discard text.

Stopping or failing a turn moves unsent queued messages to separate Paused rows.
The current composer stays separate, with its text and caret intact. Explicit
resume has the same review state; paused messages never run automatically. The
session store autosaves the composer, individual paused drafts, the queue and
prompt history. Normal insertions are limited to 1 MiB. See
[SESSIONS.md](../usage/SESSIONS.md).

A successful turn briefly shows its duration and tool count in the feedback
area. Errors and interruption remain in the journal/export. The animated row
disappears after completion or cleanup.

## Editing keys

| Key | Action |
| --- | --- |
| Enter | Send while ready; queue while generating. |
| Ctrl+J | Insert newline. |
| Shift+Enter / Alt+Enter | Insert newline when the host reports distinct keys. |
| Left / Right | Move by character. |
| Ctrl+Left / Right | Move by word. |
| Home / End, Ctrl+A / E | Start/end of the logical line. |
| Ctrl+Home / End | Start/end of the entire draft. |
| Up / Down | Move between visual rows in the editor. |
| Ctrl+P / N | Browse up to 50 dispatched prompts when no menu is open; commands are excluded. |
| Page Up / Down, mouse wheel | Scroll the conversation; an open information panel/menu receives wheel events. |
| Alt+Home / End | Conversation beginning / follow latest output, including through panels. |
| Alt+T | Enter or leave tool inspection; opens the newest call. |
| Up / Down during inspection | Select the previous/next tool; reveal its header when outside the viewport. |
| Enter during inspection | Expand or collapse retained details without sending the draft. |
| Esc / Tab during inspection | Return to the draft, leaving active work running. |
| Backspace / Delete | Delete before/after the caret. An attachment element is removed whole. |
| Ctrl+Backspace / Delete, Ctrl+W | Delete by word as reported by the host. |
| Ctrl+U | Delete back to the logical line start. |
| Alt+Up, `/drafts` | Open the pending drafts list, including during work. |
| Alt+V | Attach the clipboard image; Ctrl+V stays the host's text paste. |
| File drop, `/attach PATH...` | Attach files at the caret. See [ATTACHMENTS.md](../usage/ATTACHMENTS.md). |
| Ctrl+D in `/drafts` | Mark only the selected draft for discard; Enter confirms, Esc cancels. |
| Ctrl+S in `/drafts` | Explicitly send or queue a paused draft from the list or its editor. |
| Esc | Cancel a draft edit or close a panel first; while reading normally, return to the bottom; otherwise stop work. |
| Ctrl+C | Cancel a draft edit or close a panel; otherwise stop work or clear a draft. Never exit. |
| Ctrl+Q | Official exit shortcut: cancel and clean up active work, save input, then quit. |
| F1 | Open temporary command/keyboard help while idle. |

Ctrl+O remains inactive. Typing or editing leaves tool inspection and applies
that input to the preserved draft/caret. Page keys and the wheel keep browsing
the conversation during inspection. Prompt-history browsing
preserves the unsent draft and caret. Submitting a recalled prompt restores that
original composer; editing a recalled prompt also keeps its original for
recovery as a separate paused draft after a restart. Paste preserves newlines
and tabs; a native Windows console text burst is treated as paste, so an
extremely fast Enter may insert a
newline and require another Enter to send. Ctrl+J is the reliable newline key.
Unix terminals use bracketed paste markers to distinguish pasted text from keys.
Use a terminal with bracketed paste support for multiline pastes. Unix raw mode
disables XON/XOFF so Ctrl+Q reaches Jecode. Arrow/function/modifier keys use VT
sequences; terminal host bindings may intercept them, and legacy terminals may
report Shift+Enter exactly like Enter. Standalone Esc has a 150 ms ambiguity
window to distinguish it from a multi-byte key sequence.
Terminal host bindings may consume other combinations before Jecode sees them.
Cell widths are approximated for common Unicode; complex emoji can differ from
the terminal's actual grapheme rendering.
