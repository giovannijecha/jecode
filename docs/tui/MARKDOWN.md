# Markdown responses

The TUI styles a practical subset of Markdown in assistant responses. The
original response text remains available in the conversation archive,
`/copy`, and `/export`; single-prompt and `--plain` output also use the
original text. Terminal styling does not change what the model said.

## Supported display syntax

| Source | Terminal display |
| --- | --- |
| `#` through `######`, or a one-line Setext heading | Emphasized heading text. Optional closing hashes are removed. |
| `**strong**`, `__strong__`, `*italic*`, `_italic_`, `~~strike~~` | Styled text; styles can combine. |
| Matching backtick runs | Inline code with literal spaces and punctuation. |
| `[label](destination)` | Styled label followed by the visible destination in parentheses. Jecode does not open it. |
| `-`, `*`, `+`, or numbered list items | A hanging indent when the item wraps. `[ ]` and `[x]`/`[X]` render as task boxes. |
| `>` quote lines | A muted quote with a vertical guide. |
| Three or more matching backticks or tildes | A full-width code panel with a language label. |
| Three or more matching `-`, `*`, or `_` markers | A horizontal rule. |

Fenced Rust, shell, JSON, JavaScript/TypeScript and XML/HTML code gets light
syntax highlighting. Common file extensions select the same display lexer
inside tool output. Other languages remain readable plain code. Highlighting is
for display, not syntax validation. An unfinished fence stays open while a
response streams.

Each source line remains a separate display line, wrapping to fit the
terminal. Explicit blank lines remain visible. Consecutive prose lines are
not merged into paragraphs. Backslash escapes protect ASCII punctuation;
unmatched formatting markers stay visible as text.

## Tables

A pipe-separated header followed by a matching dash delimiter becomes a
borderless table. Optional outer pipes and left, center, and right alignment
markers work:

```markdown
| Item | State | Count |
| :--- | :---: | ---: |
| Build | Ready | 3 |
```

Use `\|` for a literal pipe in a cell, including inside inline code. Cells
wrap when needed. If columns cannot fit, each body row becomes a sequence of
`Header: value` fields; widening the terminal reconstructs the columns from
the original text. Missing cells stay empty. Extra cells remain visible in
the last column. An invalid delimiter stays ordinary text.

A blank line or a new heading, list, quote, rule, or code fence ends a table.
Tables inside code fences stay literal. A table's widths may shift while its
response is streaming and settle when the message completes.

## Display limits

This is a display subset of Markdown, not a complete CommonMark or GFM
parser. Lazy quote continuation, deeply nested block containers, reference
links, images, HTML, entities, and math have no special rendering. Their
source remains visible. Links show their destination but are not clickable
through Jecode. Terminal control characters are sanitized for display.

Common combining marks, wide CJK characters, and emoji sequences stay
together when wrapping. Exact glyph width, italics, and strikethrough depend
on the terminal and font. Resize reflows the display from source; see
[Scrolling and resize](RESIZE.md).
