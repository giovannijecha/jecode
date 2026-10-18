# Jecode theme

The native TUI uses semantic colour roles and explicit component surfaces.
Ordinary text and the conversation background inherit the terminal defaults.
The renderer emits true-colour VT sequences; the terminal host supplies the
font, glyph metrics and support for bold, italic, underline and strike.
The implementation lives in [theme.rs](../../src/tui/theme.rs).

## Colour roles

| Role | Colour | Use |
| --- | --- | --- |
| `BODY` | Inherited | Assistant prose, draft text and tool output |
| `EMPHASIS` | `#DCE4F0` | Bold welcome name, tool names, activity label and Markdown emphasis |
| `STRONG` | `#E2E8F0` | User text, panel titles, menu names and Markdown headings |
| `CODE_TEXT` | `#C9D1D9` | Unhighlighted code |
| `MUTED` | `#9AA4B2` | Descriptions, footers, information rows, quotes and notices |
| `TOOL_DIM` | `#879AAA` | Activity timer and tool inspection hint |
| `FAINT` | `#455769` | Tool guides and quote bars |
| `ACCENT` | `#7AA2F7` | `>_`, composer rules, focus, search matches, links and inline code |
| `TOOL_ACCENT` | `#99B8D4` | Running or selected tool targets and guides |
| `KEYWORD` | `#BB9AF7` | Code keywords |
| `GOOD` | `#9ECE6A` | Successful tools, code strings and additions |
| `BAD` | `#EF8B8B` | Failed tools, destructive row names and removals |
| `WARNING` | `#E0AF68` | Warnings, cancellations and code numbers |

Component aliases derive from these roles. Weight and slant are applied where
used: tool names use `EMPHASIS.bold()`, while feedback uses italic status text.
Panel titles and Markdown headings use `STRONG` without added bold.

## Surfaces

| Surface | Colour | Use |
| --- | --- | --- |
| Terminal | Inherited | Conversation and notices |
| `USER_SURFACE` | `#282E39` | Sent requests and command, selector, information and process panels |
| `SELECTED_SURFACE` | `#353D4B` | Selected menu and process rows; back-to-bottom badge |
| `INSET_SURFACE` | `#202933` | Running tool headers |
| `TOOL_SELECTED_SURFACE` | `#2A3848` | Inspected tool headers |
| `CODE_SURFACE` | `#20252C` | Fenced code |
| `DIFF_ADDED_SURFACE` | `#1E2E23` | Added replacement lines |
| `DIFF_REMOVED_SURFACE` | `#312026` | Removed replacement lines |
| `CURSOR` | `#1A1B26` on `#C0CAF5` | Static software caret |

Tool guides explicitly restore the terminal background beside shaded headers
and diff lines. The caret keeps its own fill inside every component.
Colours are designed for a dark terminal; inherited text and background depend
on the host. The palette does not impose a global terminal theme.

## Conversation and tools

One neutral row separates visible blocks. User messages have a two-cell indent
and a coloured blank row above and below. Assistant text has no automatic
vertical margins. Code starts at the left edge, including its language row;
completed fences add one coloured blank row at the bottom.

Tool headers read left to right as the branch, bold name and target, followed
by a status glyph, outcome and elapsed time. At 72 columns and wider, status
and time align on the right; narrower headers wrap the target and put status
on its own row. The glyph is `✓` for success, `×` for failure, `■` for a stop,
`▶` for background work, and blank while running. Durations remain visible,
including completed calls below one second.

The latest call previews up to three output rows. Earlier successful calls
collapse; failures and cancellations keep their preview. Header metadata can
show the retained preview extent when space permits. Bash uses the stdout tail
and gives stderr priority on failure. Reads and writes use the beginning.
Edit cards show removed text followed by added text, with replacement surfaces;
they do not calculate an additional contextual diff or hunk coordinates.
Expanded details show the retained content. Capture limits, partial reads and
errors remain visible. Presentation never changes model input or saved results.

Alt+T selects the latest tool, arrows navigate calls, Enter toggles details
and Esc returns. The inspection controls have a separate `TOOL_DIM` row above
the composer. Selected headers use the tool selection surface, not the menu
selection surface.

## Panels, controls and feedback

Panels share the user surface, two-cell side margins from 12 columns and
optional top/bottom padding. A title and visible range share a row. Menu option
descriptions start after that row's name; search matches use the accent.
The current model, effort or conversation is named in its description.
Deletion changes the row's name and `Delete?` or `Discard?` prefix while its
description remains muted. The selected surface remains in place.

Hints use each component's compact title-cased vocabulary, such as
`↑↓ move · Enter select · Esc close`, and choose a shorter complete variant
when space runs out. Numbered selector shortcuts remain available for lists of
up to eight options. Process rows use a status column and command instead of
selector numbering. Stopping a process opens a dedicated confirmation panel.

Notices are italic text at the terminal's left edge, without severity prefixes:
muted for information, gold for warnings and red for errors. They wrap by words
and retain their explicit line breaks. Information panels use muted labels and
values in one column pair when possible, or stacked rows on narrow terminals.
The process output view shows status/id, then stream controls and follow mode;
the active stream is accented.

The activity row uses four fixed blue bullets, a bold label, elapsed time and a
right-aligned Esc hint when the whole hint fits. The wave advances every 120 ms;
the label and caret stay static. Queue previews precede notices and activity.
Contextual draft hints occupy their own row above the directory/model footer.
See [COMPOSER.md](COMPOSER.md) for controls, lifetimes and queue behaviour.

## Verification

Ignored specimen tests paint the native component renderers:

```text
cargo test specimen::specimen -- --ignored --nocapture
cargo test specimen::tools -- --ignored --nocapture
cargo test specimen::composer -- --ignored --nocapture
cargo test specimen::menus -- --ignored --nocapture
cargo test specimen::messages -- --ignored --nocapture
cargo test specimen::notices -- --ignored --nocapture
cargo test specimen::panels -- --ignored --nocapture
```

Layout tests check styles, surfaces, wrapping, focus, previews and small
viewports. Native terminal checks cover input, resizing and restoration;
font-specific glyph appearance remains a terminal-host concern.
