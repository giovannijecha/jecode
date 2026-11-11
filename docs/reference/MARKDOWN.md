# Markdown display

The TUI formats the Markdown in the agent's answers for the terminal. This is
display only: the original text is what Jecode saves, exports and copies with
`/copy`, and `--plain` and single prompts print it unchanged.

## Supported syntax

| Markdown | Display |
| --- | --- |
| `#` to `######` headings, or Setext underlines | Bold text without the markers; level 1 in the accent color, level 2 brighter, deeper levels in the body color. |
| `**bold**`, `__bold__`, `*italic*`, `_italic_`, `~~struck~~` | Bold, italic and dimmed struck text; styles can combine. |
| `` `code` `` | Inline code on a code background. |
| `[label](https://example.com/page)` | The underlined label followed by the host in parentheses, `(example.com)`. Relative links stay whole. Links are not clickable. |
| `-`, `*`, `+` and numbered lists | Bullets or numbers with hanging indents. `[ ]` and `[x]` show as `☐` and `☑`. |
| `>` quotes | Muted text behind a `│` guide. |
| Fenced code blocks (three or more backticks or tildes) | A code card with the language name and syntax highlighting. |
| `---`, `***`, `___` | A horizontal rule. |
| Tables | A boxed table. |

Consecutive lines form one paragraph that reflows to the terminal width, as in
CommonMark. A blank line or the start of another block ends it. Two trailing
spaces or a trailing backslash keep a line break. Backslash escapes protect
punctuation, and unmatched markers stay visible as text.

## Code blocks

Code is highlighted for keywords, strings, numbers and comments in Rust,
JavaScript and TypeScript, JSON, shell, CSS, SCSS and LESS, Python, YAML, TOML
and INI, Go, C, C++, C#, Java, Kotlin, Swift, PowerShell, SQL, Perl, Ruby, Lua,
PHP, batch files, Markdown and XML, HTML and SVG. The fence's language label or a
file extension chooses the language; others stay plain. Highlighting is for
reading, not validation. The same highlighting applies to code in tool cards.

Long code lines wrap instead of being cut: after spaces and separators such as
`,`, `;` or `{`, and within a word only when it is longer than the row.
Continuation rows are indented under the line's own indentation. `/copy`
copies the original code, not the wrapped rows. While an answer streams, an
unfinished code block stays open.

## Tables

```markdown
| Item | State | Count |
| :--- | :---: | ---: |
| Build | Ready | 3 |
```

A header row followed by a matching delimiter row becomes a table with borders;
left, center and right alignment markers work, and outer pipes are optional.
Use `\|` for a literal pipe in a cell. When a table is too wide, its widest
columns narrow and cells wrap; if even that does not fit, each row is shown as
`Header: value` lines, and widening the terminal brings the columns back.
Missing cells stay empty and extra cells stay visible in the last column. A
table's widths can shift while the answer streams.

## Limits

This is a practical subset, not a complete CommonMark or GitHub Flavored
Markdown renderer. Deeply nested blocks, reference links, images, HTML, entities
and math are shown as their source text. Terminal control characters are
removed from the display. Combining marks, wide CJK characters and emoji
sequences wrap correctly, but their exact width and the look of italics and
strikethrough depend on your terminal and font.
