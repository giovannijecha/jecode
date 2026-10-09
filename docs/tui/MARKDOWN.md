# Markdown responses

The TUI renders model responses using original Rust code and the standard
library. Rendering is separate from the agent loop. Messages retain their
original Markdown in the conversation, archive, `/copy` and `/export`; plain
mode and single-prompt output also keep the original text.

## Tables

A pipe-separated header followed by a matching dash delimiter becomes a table
with clean columns, two spaces between columns and a rule below the header.
There are no enclosing borders. Optional outer pipes and left, center and right
alignment markers are supported. Use `\|` for a literal pipe, including inside
an inline code span.

```markdown
| Dimension | Rating | Note |
| :--- | :---: | --- |
| Speed | High | Direct tools |
| Maintenance | Good | Small owned modules |
```

Widths follow the visible contents of the current response snapshot. Cells wrap
when their natural widths exceed the terminal width. When usable columns cannot
fit, each row becomes a sequence of `Header: value` fields. Widening the terminal
reconstructs the columns from source. Missing body cells remain empty; surplus
body cells stay visible in the final column instead of being discarded.

A blank line or a new heading, list, quote, rule or code fence ends a table.
Tables inside code fences stay literal. An invalid delimiter stays ordinary
text. During streaming, a table can change widths as cells arrive; these layouts
remain provisional until the message completes.

## Text and blocks

Supported display syntax includes:

- ATX headings (`#` through `######`), optional closing hashes, and single-line
  Setext headings with an `=` or `-` underline.
- Strong emphasis with `**` or `__`, italic emphasis with `*` or `_`, combined
  emphasis and strikethrough with `~~`.
- Inline code with matching backtick runs. Its text and spaces stay literal,
  including when the line wraps.
- Inline links in the form `[label](destination)`. The styled label is followed
  by the visible destination in parentheses. Rendering does not open links.
- Unordered and numbered list items with hanging indentation; task markers
  `[ ]`, `[x]` and `[X]` appear as unchecked or checked boxes.
- Explicit quote lines, horizontal rules and matching backtick or tilde fences.
  Rust, Bash and JSON fences have light syntax highlighting.
- Backslash escapes for punctuation. Unmatched formatting markers remain visible.

Unsupported code languages stay plain. The highlighter is a display lexer,
not a syntax validator.

This is a display subset, rather than a complete CommonMark/GFM parser.
Paragraph continuation, deeply nested block containers, reference links,
images, HTML, entities and mathematics have no specialized rendering. Unsupported
syntax remains visible as text. Terminal controls are sanitized for display.

## Width and responsiveness

Widths account for common combining accents, wide CJK characters, emoji
modifiers, flags, keycaps and emoji joined with a zero-width joiner. These groups
remain whole during wrapping, clipping, caret drawing and ordinary cursor edits.
The width model is approximate: the terminal and font determine the final glyphs.
Italic and strikethrough also depend on the terminal's style support.

Settled table layout measures one source row per iterator step, then emits at
most 32 wrapped visual rows per step. A long cell retains wrap state between
steps rather than allocating all its visual rows at once. Measurement and
emission count toward the transcript's layout allowance. Parsing an individual
source row remains linear in its input size; live previews render the current
snapshot. See [RESIZE.md](RESIZE.md) for source reflow and budgets.

## Verification

Synthetic tests cover clean columns and alignment, escaped pipes, missing and
surplus cells, narrow wrapping and vertical fields, styles, literal fences,
partial delimiters, long tables and tall cells. Integration fixtures exercise
cumulative streaming, finalization and narrow/wide reconstruction through the
transcript and VT screen model. They preserve source, draft and queue and check
that provisional layouts leave no duplicate rows in the owned viewport or the
caller's native scrollback. Reading anchors keep wrapped text across resize,
including multiline table headers.
