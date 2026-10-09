# Jecode theme

The palette is shared by the composer, conversation, tool cards and code
highlighter. Ordinary text and the general background inherit the terminal's
defaults. Explicit backgrounds belong to user messages, active/selected tools,
code, diffs, the composer's steady block caret and panels.

The implementation lives in [src/tui/theme.rs](../../src/tui/theme.rs).

Startup and new conversations show `>_ Jecode`: the `>_` marker uses the blue
accent and the name uses strong emphasis.

| Element | Foreground | Background |
| --- | --- | --- |
| Blue accents, links and inline code | `#7AA2F7` | Inherited |
| Purple keywords | `#BB9AF7` | Inherited |
| Successes and strings | `#9ECE6A` | Inherited |
| Errors | `#EF8B8B` | Inherited |
| Warnings and numbers | `#E0AF68` | Inherited |
| Running/selected tool text | `#99B8D4` | `#202933` / `#2A3848` |
| Tool guides | `#455769` | Inherited |
| Activity timer and hints | `#879AAA` | Inherited or panel surface |
| Secondary text and comments | `#9AA4B2` | Inherited |
| Headings and strong emphasis | `#DCE4F0` | Inherited |
| User messages | `#E2E8F0` | `#282E39` |
| Command and menu panels | Light and secondary text | `#282E39` |
| Transient information / warning / error | `#9AA4B2` / `#E0AF68` / `#EF8B8B` | Inherited |
| Selected command/menu row | `#E2E8F0` | `#353D4B` |
| Code blocks | `#C9D1D9` | `#20252C` |
| Added diff lines | `#9ECE6A` | `#1E2E23` |
| Removed diff lines | `#EF8B8B` | `#312026` |
| Composer caret | `#1A1B26` | `#C0CAF5` |

Code tokens use their semantic foreground over the code block background.
Markdown emphasis combines bold, italic and strikethrough styles. Link labels
are blue and underlined; their visible destinations use blue. Table headers use
strong emphasis and their separator uses secondary gray. Column padding inherits
the terminal background. See [MARKDOWN.md](MARKDOWN.md) for layout behavior.
Colored blank rows belong to user/code panels; neutral blank rows separate
conversation blocks. User panels retain one colored padding row above and below.
Code panels have a language row above and an empty colored row below. Tool trees
have no internal spacer rows. Local command output uses temporary UI panels.
See [COMPOSER.md](COMPOSER.md#conversation-spacing) for the spacing contract.
Both composer borders and the input marker use the same blue accent at rest,
and secondary gray during generation. Placeholder and footer use secondary gray;
the activity label uses strong emphasis. The caret is a software-drawn
rectangle with no blinking; it follows the editing position and preserves the
character underneath. The native cursor is restored when leaving the TUI.

Commands and selectors share a borderless `#282E39` panel, including titles,
input, options and keyboard controls. The ordinary draft composer
retains its blue/gray borders. The light block caret keeps its own background
inside shaded panels.
Information panels use the same surface and two-cell indent. Transient feedback
uses italic text aligned at the terminal's left edge, with an inherited
background even while a command or menu panel is open. Long feedback wraps to
the same left edge. Feedback has no status dot; its text color indicates severity.
Short results disappear after five seconds or editing, while errors wait for
the next user action. Loading appears once in its operation title. Help and
temporary-file panels stay open until dismissed, and none of these rows become
saved terminal history.
Selected command/selector rows use `#E2E8F0` on `#353D4B` across the row, with a
blue `›`, blue matched characters and gray descriptions. Four identical round
activity bullets use a blue brightness wave, aligned at the left edge above the
composer. Glyph position and label color stay fixed; the software caret stays
static. See [COMPOSER.md](COMPOSER.md) for timing and interaction rules.

## Tool status legend

Tool cards form an inline tree. `├─` connects successive calls and `└─` closes a
sequence. Quiet guides (`#455769`) sit at the terminal's left edge and inherit
its background, including beside shaded previews. Each header has a bold real
tool name, command or path, followed by a right-aligned textual outcome and live
duration. Running/selected guides use the tool accent. Calls remain separate,
without tool numbers, triangles or animated header markers.
Consecutive calls stay connected while the model processes their results. The
sequence closes at a visible conversation message or when the turn ends.

| Status | Meaning | Example outcome |
| --- | --- | --- |
| Blue `#99B8D4`, text only | Execution in progress | `running` |
| Green `#9ECE6A`, `✓` | Execution completed successfully | `exit 0`, `lines 1–20` |
| Red `#EF8B8B`, `×` | Execution failed or timed out | `failed`, `exit 7`, `timed out` |
| Gold `#E0AF68`, `■` | Execution cancelled | `cancelled` |

The textual outcome accompanies the color. A successful command whose captured
output was truncated keeps its green execution marker and adds a gold notice
explaining the capture limit.

The latest call in a sequence shows up to three preview content rows; earlier
successes collapse to headers. Failures and cancellations retain a preview unless explicitly collapsed.
Alt+T selects the latest tool, Up/Down navigates calls and Enter opens or closes
retained details in place. Selection uses `#2A3848`; running headers use `#202933`.
Live durations stop at completion; historical resumed tools omit unavailable timings.
Tool presentation does not change model input.
Bash shows the retained output tail and keeps stderr visible; failures prioritize
stderr. Reads and writes show the beginning. Edits balance actual removed and
added text without generating hunk coordinates. When a preview omits content,
its header adds muted metadata such as `preview 3 / 8`; no omission row interrupts
the content. Expanded details remove this metadata. Narrow headers omit it when
it cannot fit. Capture-limit and partial-read notices remain separate because
they describe the available data. The configured capture limits still apply;
`/export` includes the retained calls and results beyond the preview.

Cards remain in the owned fullscreen conversation. Results and final tree
branches update their existing cards. The mouse wheel and Page Up/Down browse
the same conversation viewport; new output preserves an older reading position.
Width changes reflow source while preserving the palette and spacing.
Below 72 columns, commands wrap and status/duration move to a separate row.
Expanded output wraps to the viewport; wide command rows and compact previews
shorten with `…`. Original retained data also remains available in the export. See
[RESIZE.md](RESIZE.md) for reflow and scrolling behavior.
