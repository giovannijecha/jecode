# Composer and conversation controls

The shared Windows, Linux and macOS TUI uses the terminal's alternate buffer.
One conversation column scrolls above the lower area. The lower area contains
pending drafts, feedback, activity, inspection controls and the composer, or
an open command, selector, information or process panel.

Page Up/Down and the mouse wheel browse the conversation. Reading older content
pauses following new output. Alt+Home reaches the start and Alt+End follows the
latest output, including through open panels. A blue Back to bottom badge marks
paused reading. Esc returns to the bottom during ordinary reading; panels,
draft editing, history and tool inspection receive Esc first.

Resize preserves the reading anchor, logical draft/caret, selector query and
selection, and queued drafts. Reflow starts after 75 ms without another size
change; generation and input continue between layout passes. Exit restores the
caller's main screen, scrollback and terminal modes, then prints a short summary
and saved-session resume command. `--plain` keeps the line-oriented CLI.
See [RESIZE.md](RESIZE.md).

## Conversation spacing

Visible blocks have one neutral separator, with none before the first block.
A connected tool tree is one block: successive headers and previews remain
adjacent. Empty assistant updates do not create blocks or close a tool tree.
Local command results and feedback use temporary UI rows rather than transcript
rows; audit records remain available in saved sessions and exports.

User messages have a two-cell indent, their own literal line breaks and one
coloured padding row above and below. Assistant Markdown keeps source line
breaks and explicit blank rows. Blocks have no automatic vertical margins.
Fenced code occupies the full row width: a language row, code rows and, after
a closing fence, one coloured blank row. An open fence has no closing row.
See [MARKDOWN.md](MARKDOWN.md).

One neutral row normally separates the visible conversation from the lower
area. Below two available rows it is omitted to keep context and input reachable.
The lower area does not add internal spacer rows except a component's own
padding, contextual hint or footer.

## Draft and footer

The idle composer has four rows: a full-width top rule, input, bottom rule and
directory/model/effort footer. The rules and `›` marker use blue at rest and gray
during work. Entered text inherits the terminal foreground; the empty draft
shows the gray `Ask anything…` placeholder.

A static light software caret covers the character at the insertion position,
or a space at the end. Empty drafts put it over the placeholder's first letter.
The terminal's native cursor stays hidden until exit.

At most five consecutive visual rows follow the caret through long drafts.
Newlines and soft wrapping both count as visual rows. The top rule names the
visible range only while some draft text is hidden. It can also name draft
editing, prompt history or pending drafts. Attachment labels are indivisible
editing units; the editor preserves Unicode glyph clusters.

Editing a queued/paused draft and browsing prompt history add a separate hint
row above the ordinary footer. The directory, model and effort remain visible
when space permits:

```text
─ Editing draft 2/2 ──────────────────────────────────────
› Plan for the follow-up:█
─────────────────────────────────────────────────────────
Enter save · Ctrl+S send · Esc cancel
C:\project                                   model · high
```

Directory text shortens to leave the model and effort on the right. Tiny
terminals drop supporting rows and retain the input/caret.

### Context indicator

The footer's right group leads with the engine's context projection, in the
same muted footer style:

```text
C:\project              Context ≈61% · Auto 80% · model · high
```

`≈61%` is the estimated input of the next model request as a share of the
effective context window, including any ceiling learned from provider
rejections; `<1%` marks a small nonzero share. `Auto 80%` is the configured
**Auto-compact at** threshold. Unsubmitted draft text is not counted. While the
model's capacity is unknown, at startup before the catalog is read and after a
model change or key replacement, the indicator reads `Context unknown` and never
shows a percentage or a previous model's value.

The indicator follows the engine: capacity becoming known, accepted input,
completions with their usage, tool results, previews, compaction, provider
context recovery, settings changes, new and resumed conversations, and model
switches. After compaction it shows the reduced estimate. It stays visible during
work.

As width shrinks, the directory keeps up to 12 cells; then the indicator drops
`Auto 80%`, then shortens to `≈61%` (`Context ?` when unknown) and finally the
model text is ellipsized after it. Open panels keep the footer below their
shaded surface when the panel has at least ten rows; shorter panels give that
row to their input, selection and controls.

Windows console
input handles typing immediately and groups detected multiline paste across
native reads. Ctrl+J is the portable newline; Shift/Alt+Enter depend on the host.

## Commands and selectors

A leading `/` opens a shaded command panel in place of the bordered composer.
Its title, input, suggestions and controls share the user surface. Panels have
two-cell side margins from 12 columns and coloured top/bottom padding when it
fits. The selected row uses a lighter surface and blue `›`; matches are blue,
option names light and descriptions gray. Descriptions follow each row's name,
with a small aligned minimum column when space permits. Long names take priority.

Command suggestions show up to six matches. Matching is case-insensitive and
ranks contiguous matches before scattered letters. Up/Down selects, Tab completes
the name, and Enter runs it when idle. During work, Enter queues the written
command except `/copy`, `/drafts` and `/terminal`, which open immediately.
Whitespace closes suggestions while retaining command input. Esc dismisses the
panel without clearing text or moving the draft caret.

Model, effort, settings, session, draft and copy selectors share the panel
geometry. Up to eight options use numbers; larger lists have a separate search
field and up to eight visible matches. An empty search field has a blank caret.
The title shows a visible range or compact match count when needed. Current
values appear in descriptions, such as `· current` or `Current selection`.
They are independent of keyboard focus.

Deletion/discard marks the selected name red with `Delete?` or `Discard?`;
descriptions stay gray and the selected surface remains. Enter confirms,
Esc cancels, and searching or navigating cancels the mark. The selected row,
caret and controls take priority over padding in short viewports.

- `/model [ID]` selects a tool-capable model and then an effort; both apply
  together. `/effort [NAME]` can change effort directly. These choices affect
  this conversation and retain the original content/tool pairs.
- `/settings` changes saved defaults, Auto-compact at or the OpenRouter key.
  Default model and effort apply on `/new` and future launches. Key entry is
  masked, validated in the background and saved only on success. Esc in a child
  picker returns to settings.
- **Auto-compact at** edits a whole percentage from 1 to 99 (default 80), with a
  help row noting that higher values leave less room for output. Enter saves;
  an invalid entry shows a warning and keeps the panel and previous value. A
  successful save applies to this conversation and new ones and updates the
  footer; a failed save or Esc keeps the previous value.
- `/resume [ID]` restores a saved conversation in this folder. Ctrl+D marks
  one for deletion. Deleting the current conversation starts fresh context while
  preserving unsent composer, queued and paused drafts. The list remains open.
- `/drafts` or Alt+Up opens pending drafts during a turn too. Queued rows precede
  paused rows. Enter edits one; Ctrl+D marks discard; Ctrl+S explicitly sends a
  paused draft. The main composer and caret survive every operation.
- `/copy` selects completed original response text, code or quotes, preserving
  original line breaks and indentation. Its feedback has an independent slot.
- Unknown commands report local errors and possible spelling corrections.
  `/clear`, `/setup`, `/exit` and `/quit` remain compatibility commands.

## Feedback and information

Notices are italic text at the terminal's left edge on the inherited background:
muted for results, gold for warnings and red for errors. They have no added
severity glyph and wrap by words. Brief notices disappear after five seconds
or when editing starts. Errors remain until the next user action. Progress
remains until its operation clears or replaces it. Copy feedback remains
independent of other notices. None of these rows enter saved transcript layout.

`/help` and `/tmp` replace the composer with an information panel. Their muted
label/value rows align when the longest label fits comfortably; otherwise labels
and values stack at full content width. `/help` lists commands followed by keys.
The title shows the visible range. Up/Down scrolls, Page Up/Down pages and
Home/End reaches either end. Esc, Ctrl+C or Enter closes without sending;
typing returns to the preserved draft and applies that input.

## Processes

`/terminal [ID]` opens active processes or saved output, including during a turn.
The list uses a status column followed by the command, with the usual focus
marker and selected surface. Enter opens the selected process. The output view
shows the command title, status/id, then Output/Errors controls and follow mode.
The active stream is blue. Tab switches streams; arrows, Page Up/Down and the
wheel browse output; End follows the tail. Draft and queue remain intact.

Ctrl+D opens a dedicated stop confirmation naming the command, status and id.
Enter stops that process and its descendants; Esc or Ctrl+C cancels. A process
that exits while confirmation is open becomes an already-finished confirmation.
At one row high, the confirmation title remains visible. `/terminal stop ID`
requests the same local stop directly. Model-side output cursors remain separate
from panel inspection.

## Activity and queue

A turn shows four fixed blue bullets, a bold phase label, elapsed time and a
right-aligned Esc hint when it fits. The brightness wave advances every 120 ms.
Elapsed time spans the entire turn, with tenths below a minute and `1m 04s`
afterward. Waiting, thinking, tools and streamed text reflect actual events.
The caret, label and tool headers do not animate.

```text
› Also check the process panel.                            queued
••••  Running tools  ·  12.4s                         Esc stops
──────────────────────────────────────────────────────────────
› next draft█
──────────────────────────────────────────────────────────────
C:\project                                        model · high
```

Queue previews precede the feedback/activity rows and show the first logical
line, adding `…` for multiline text. Short viewports keep the next message
visible, with range metadata when it fits. An open Drafts list replaces those
previews. Tool inspection adds its own controls above the composer.

The draft remains editable while working. Enter holds up to eight pending
messages, clearing input only on success. Ordinary messages join the current
turn before its next model request; local commands wait until that turn ends.
Stopping or failing pauses messages that have not joined the turn. Explicit
resume also requires reviewing paused drafts; they never run automatically.
Autosave preserves the composer, queue, paused drafts and dispatched prompt
history. A successful turn briefly shows elapsed time and tool count.

## Editing keys

| Key | Action |
| --- | --- |
| Enter | Send while ready; while working, join the active turn before its next model request. |
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
| Ctrl+S in `/drafts` | Explicitly send a paused draft from the list or its editor; during work it joins the active turn. |
| `/terminal [ID]` | Open active processes or a process's output, including during work; Tab switches Output/Errors, End follows, Ctrl+D marks a stop and Enter confirms. |
| `/terminal stop ID` | Request a stop for one process in the current conversation. |
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
