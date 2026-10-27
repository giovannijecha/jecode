# Markdown responses

The TUI styles a practical subset of Markdown in assistant responses. The
original response text remains available in the conversation archive,
`/copy`, and `/export`; single-prompt and `--plain` output also use the
original text. Terminal styling does not change what the model said.
In the conversation, an accent `●` opens each assistant response. Wrapped
rows and later blocks hang two cells in; the dot is presentation, not part of
the archived or copied response. Colours follow the [theme](THEME.md).

## Supported display syntax

| Source | Terminal display |
| --- | --- |
| `#` through `######`, or a paragraph followed by a Setext underline | Bold heading text without the markers: level 1 (and `===`) in the accent, level 2 (and `---`) strong, deeper levels in the body colour. Optional closing hashes are removed, and one blank row follows a heading that touches the next line. |
| `**strong**`, `__strong__`, `*italic*`, `_italic_`, `~~strike~~` | Bold text in the strong colour, italics, and struck text dimmed; styles can combine. |
| Matching backtick runs | Inline code on the code surface, with literal spaces and punctuation. |
| `[label](destination)` | The underlined accent label, then the destination's host in muted parentheses, such as `(docs.rs)`; a relative or bare destination stays whole. Jecode does not open it. |
| `-`, `*`, `+`, or numbered list items | A muted `•` or number with a hanging indent when the item wraps. `[ ]` and `[x]`/`[X]` render as `☐` and a green `☑`. |
| `>` quote lines | A muted quote behind a `│ ` guide. |
| Three or more matching backticks or tildes | A code card on the code surface across the response width: the language label muted on its top row, code one cell in, and a closing surface row once the fence closes. Long lines wrap at token boundaries, never truncated, with continuations hung under their indentation. |
| Three or more matching `-`, `*`, or `_` markers | A full-width faint rule. |

Fenced code gets light syntax highlighting, keywords, strings, numbers and
muted comments, from one display lexer driven by language profiles: Rust,
JavaScript/TypeScript, JSON, shell, CSS/SCSS/LESS, Python, YAML, TOML/INI, Go,
the C family (C, C++, C#, Java, Kotlin, Swift), PowerShell, SQL, Perl, Ruby,
Lua, PHP, batch files, Markdown and XML/HTML/SVG. The fence label or a file
extension selects the profile, so tool output uses the same lexer. Other
languages remain readable plain code. Highlighting is for display, not syntax
validation. An unfinished fence stays open while a response streams.

Code rows, in code cards and file cards alike, break after a space or a
separator such as `,`, `;`, `{` or `)`, then before `.` or `=` when nothing
better fits, and inside a token only when it is longer than the row; a row
never opens with `,`, `;`, `)`, `]` or `}`. Markup prefers a break before a
tag's `<` while the row stays at least 60% full. Continuations hang two cells
under the code's own indentation, keeping at least half the width, and drop
their leading spaces. `/copy` copies the code block's source, not its rows.

Consecutive prose lines form one paragraph, as in CommonMark: a plain line
continues the paragraph, list item or quote above it, joined by a space, so
emphasis can span the break and the text reflows to the terminal width.
Consecutive `>` lines continue one quote. A blank line or the start of a
heading, list item, quote, rule, table or code fence ends the paragraph. Two
trailing spaces or an unescaped trailing backslash keep a hard line break
inside a paragraph. The backslash is consumed only when another line continues
that paragraph; it stays literal at paragraph and block boundaries, including
the end of the document. A paragraph longer than 8 KiB continues at the next
source line. Explicit blank
lines remain visible. Backslash escapes protect ASCII punctuation; unmatched
formatting markers stay visible as text.

## Tables

A pipe-separated header followed by a matching dash delimiter becomes a
boxed table: faint `┌┬┐ ├┼┤ └┴┘ │` borders, the header in bold strong, one
cell of padding in each cell, and a `├┼┤` separator after the header and
every record. Optional outer pipes and left, center, and right alignment
markers work:

```markdown
| Item | State | Count |
| :--- | :---: | ---: |
| Build | Ready | 3 |
```

Use `\|` for a literal pipe in a cell, including inside inline code. Columns
take their natural widths; when the table is too wide the widest columns
narrow first and cells wrap. If even minimum widths cannot fit, each body row becomes a sequence of
`Header: value` fields; widening the terminal reconstructs the columns from
the original text. Missing cells stay empty. Extra cells remain visible in
the last column. An invalid delimiter stays ordinary text.

A blank line or a new heading, list, quote, rule, or code fence ends a table.
Tables inside code fences stay literal. A table's widths may shift while its
response is streaming and settle when the message completes.

## Display limits

This is a display subset of Markdown, not a complete CommonMark or GFM
parser. Deeply nested block containers, reference
links, images, HTML, entities, and math have no special rendering. Their
source remains visible. Links show only their host and are not clickable
through Jecode; `/copy` keeps the full address. Terminal control characters are sanitized for display.

Common combining marks, wide CJK characters, and emoji sequences stay
together when wrapping. Exact glyph width, italics, and strikethrough depend
on the terminal and font. Resize reflows the display from source; see
[Scrolling and resize](RESIZE.md).
