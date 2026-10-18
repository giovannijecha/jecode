# TUI theme

Jecode's interactive TUI is designed for a dark terminal. It inherits the
terminal's ordinary text and background, then applies semantic foreground
roles and component surfaces. It emits true-color VT styling; the terminal
chooses the font and controls how bold, italic, underline, and strikethrough
appear. The palette and style aliases live in
[`theme.rs`](../../src/tui/theme.rs).

## Colour roles

| Role | Colour | Current use |
| --- | --- | --- |
| `BODY` | Inherited | Assistant prose, draft text, tool output. |
| `STRONG` | `#E2E8F0` | User text, panel titles, menu names. |
| `EMPHASIS` | `#DCE4F0` | Tool names, activity labels, Markdown headings and strong text. |
| `CODE_TEXT` | `#C9D1D9` | Unhighlighted fenced code. |
| `MUTED` | `#9AA4B2` | Descriptions, footer, quotes, information and neutral notices. |
| `TOOL_DIM` | `#879AAA` | Tool inspection hint and activity time. |
| `FAINT` | `#455769` | Decorative tool guides, below text contrast. |
| `ACCENT` | `#7AA2F7` | Jecode mark, composer rules, focus, matches, links and inline code. |
| `TOOL_ACCENT` | `#99B8D4` | Active or selected tool targets and guides. |
| `KEYWORD` | `#BB9AF7` | Code keywords. |
| `GOOD` | `#9ECE6A` | Successful tools, added lines and code strings. |
| `BAD` | `#EF8B8B` | Failed tools, destructive choices and removed lines. |
| `WARNING` | `#E0AF68` | Warnings, stopped tools and code numbers. |

Weight and slant are applied by the component: tool names and Markdown
headings are bold, while notices are italic. Links use the accent with
underline. The palette does not replace the terminal's global colors.

## Surfaces

| Surface | Colour | Current use |
| --- | --- | --- |
| Terminal | Inherited | Conversation and notices. |
| `USER_SURFACE` | `#282E39` | Sent requests and command, selector, information and process panels. |
| `SELECTED_SURFACE` | `#353D4B` | Selected menu/process rows and Back to bottom badge. |
| `INSET_SURFACE` | `#202933` | Running tool headers. |
| `TOOL_SELECTED_SURFACE` | `#2A3848` | Inspected tool headers. |
| `CODE_SURFACE` | `#20252C` | Fenced code. |
| `DIFF_ADDED_SURFACE` | `#1E2E23` | Added replacement lines in edit previews. |
| `DIFF_REMOVED_SURFACE` | `#312026` | Removed replacement lines in edit previews. |
| `CURSOR` | `#1A1B26` on `#C0CAF5` | Software caret, including in shaded panels. |

## Conversation and tool cards

The conversation has one neutral row between visible blocks. Sent user
messages have a two-cell indent and shaded padding; assistant Markdown
preserves source lines and blank rows. Fenced code spans the full row width.
See [Markdown responses](MARKDOWN.md) for the display subset.

Tool calls form an inline tree. Each header has a branch, bold tool name,
target, status, and elapsed time. At widths of 72 columns or more, outcome
and time align to the right; on narrower screens, the target wraps and the
status gets its own row. Running cards use the inset surface. Success is
green, failure red, stopped work gold, and background work uses the tool
accent. The selected card uses its own blue-gray surface.

The latest call previews up to three output rows. Earlier successful calls
collapse; failures and stops keep their preview. Bash previews favor recent
stdout, with failed stderr first. Read and write previews begin at the start
of their content. Edit previews show removed then added text on separate
surfaces. Expanding a card shows retained details, including capture limits
and errors. These display choices do not alter model input or saved results.

`Alt+T` enters tool inspection, arrows choose a call, `Enter` toggles details,
and `Esc` returns to the draft. See [Composer and controls](COMPOSER.md) for
the complete interaction map and [Tools](../usage/TOOLS.md) for tool behavior.

## Panels and feedback

Commands, selectors, information, and process views share the user surface.
At 12 columns or wider, content has two-cell side margins. The selected row
uses a lighter surface and accent focus marker; current values are identified
in descriptions rather than by focus alone. Delete and discard confirmations
turn the target name red until confirmed or canceled.

Notices sit on the terminal background at the left edge: italic muted text
for information, gold for warnings, and red for errors. The activity line
uses four animated dots, a bold phase label, and muted elapsed time. Queue
previews appear above notices and activity. The context indicator stays in
the muted footer style, including below an open panel.
