# Command language

The `bash` tool runs Jecode's own small command interpreter, identically on
Windows, Linux and macOS. It never starts Git Bash, GNU Bash, `sh`, PowerShell
or any other installed shell, and never hands a script to one as a fallback.
The tool keeps the name `bash` because models know it.

Each call starts a fresh interpreter in the project directory. Changes made with
`cd`, assignments or `export` last only for that call. Programs that are not
built-in commands are launched directly by the operating system and must be
installed or present in the project.

Use `exec` instead when a program needs exact literal arguments or an
interactive terminal; see [Tools](TOOLS.md#commands).

## Syntax

- **Quoting and variables.** Single and double quotes, backslash escapes,
  `$NAME`, `${NAME}`, `${NAME-default}`, `${NAME:-default}`, `$?`, `$$`, `$!`,
  `$0`–`$9` and `$@`. `"$@"` expands to one word per positional argument.
- **Command substitution.** `$(command)` runs in a separate interpreter and
  captures up to 1 MiB of UTF-8 output, with trailing newlines removed. In an
  assignment-only command such as `x=$(command)`, `$?` is the substituted
  command's status.
- **Lists and pipelines.** `;` and newlines, `&&`, `||`, pipelines with `|` and
  background commands with `&`. A newline or comment after `&&`, `||` or `|`
  continues the command. Pipeline stages run concurrently, each in its own
  subshell. `wait` waits for background jobs started in the same call.
- **Per-command variables.** `NAME=value command` sets `NAME` for that command
  only, including built-ins, pipeline stages and background jobs.
- **Redirection.** `<`, `>`, `>>`, `2>`, `2>>`, `>&2`, `2>&1`, `&>` and `&>>`.
  Here-documents (`<<WORD`, `<<-WORD`) and here-strings (`<<< WORD`) feed
  standard input. A quoted here-document delimiter keeps its body literal;
  an unquoted delimiter expands variables and command substitutions. A descriptor
  number counts only directly before `>`, so `echo 1 > file` writes `1`. `/dev/null`
  works on every platform. A redirection alone, such as `> file`, creates or
  truncates the file. The shell does not exit until a background command has read
  its here-document or closed its standard input.
- **Control flow.** `if … then … elif … else … fi`, `for NAME in WORDS; do …;
  done`, `for ((NAME=START; NAME<LIMIT; NAME++)); do …; done`, `while` and
  `until` loops, and `( … )` subshells. They nest, accept redirections after
  their closing word (`done > file`) and work in `&&`/`||` lists and pipelines.
- **Braces.** `{}` and braces without a comma or `..` are ordinary characters,
  so `find . -exec cmd {} \;` and `HEAD@{1}` work.
- **Path patterns.** Unquoted `*`, `?` and bracket classes expand to matching
  paths. A pattern without matches stays literal; quoted patterns stay literal.
  A redirection target may be a pattern that matches one path; several matches
  are an `ambiguous redirect` error. Here-strings are not expanded as patterns.

On Windows, variable names are case-insensitive like the environment, so `$PATH`
reads `Path`.

## Built-in commands

`pwd`, `cd`, `echo`, `printf`, `cat`, `ls`, `mkdir`, `cp`, `mv`, `rm`, `touch`,
`test` and `[`, `sleep`, `env`, `printenv`, `export`, `set -e` and `set +e`,
`wc -l`, `wc -c`, `seq`, `tr -d`, `head`, `tail`, `grep`, `mktemp`, `wait`, `true`, `false`,
`:` and `exit`.

Built-ins support only the options they implement; for example, `tr` supports
only `-d`. A few behaviors to know:

- `wc` accepts one of `-l` (newline count) or `-c` (byte count), files or standard
  input, and a total for multiple files. `-` reads standard input; `--` separates
  options from filenames. It counts in chunks without loading whole files.
- `bash PATH [ARGS]` and `sh PATH [ARGS]` run a script with this same
  interpreter, in a fresh state with its own positional arguments.
- `cp -r` refuses to copy a directory into itself.
- `mv` falls back to copy and delete when moving across volumes.
- `mktemp` accepts `-d` and a template ending in at least six `X` characters,
  and never replaces an existing path.
- `head` and `tail` select lines with `-n N` or `-N`; `tail` also accepts
  `-n +N`.
- `grep` supports basic and extended regular expressions, fixed strings, common
  selection and context options, and recursive file search. Perl regular
  expressions (`-P`) are unsupported.
- `grep` prints the original bytes of a line, in any encoding, except for its
  line ending: carriage returns before the newline are neither matched nor
  printed.
- `grep -r` follows a link named as an operand and skips links and junctions
  met during the walk; `-R` follows them too and stops at loops. `--include`,
  `--exclude` and `--exclude-dir` also apply to operands other than standard
  input, and the last matching `--include` or `--exclude` wins.
- `grep` treats input holding a NUL byte as binary: a NUL also ends a line, and
  the search stops at the first match, reporting only that the file matches.
  `-a` or `--binary-files=text` searches it as text; `-I` or
  `--binary-files=without-match` treats it as not matching.

## Not supported

The interpreter is deliberately small. It does not do unquoted word splitting,
brace expansion, functions, aliases, arrays, traps, arithmetic expansion
(`$(( ))`), backtick substitution, brace groups (`{ …; }`) or interactive job
control.

Unsupported syntax and unbalanced quotes fail with an error before anything
runs. When a task needs one of these features, the model can call an installed
interpreter explicitly through `exec`, for example `python` or `pwsh`.
