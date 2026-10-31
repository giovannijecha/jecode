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
- **Redirection.** `<`, `>`, `>>`, `2>`, `2>>` and `>&2`. A descriptor number
  counts only directly before `>`, so `echo 1 > file` writes `1`. `/dev/null`
  works on every platform. A redirection alone, such as `> file`, creates or
  truncates the file.
- **Control flow.** `if … then … elif … else … fi`, `for NAME in WORDS; do …;
  done`, `for ((NAME=START; NAME<LIMIT; NAME++)); do …; done`, `while` and
  `until` loops, and `( … )` subshells. They nest, accept redirections after
  their closing word (`done > file`) and work in `&&`/`||` lists and pipelines.
- **Braces.** `{}` and braces without a comma or `..` are ordinary characters,
  so `find . -exec cmd {} \;` and `HEAD@{1}` work.

On Windows, variable names are case-insensitive like the environment, so `$PATH`
reads `Path`.

## Built-in commands

`pwd`, `cd`, `echo`, `printf`, `cat`, `ls`, `mkdir`, `cp`, `mv`, `rm`, `touch`,
`test` and `[`, `sleep`, `env`, `export`, `set -e` and `set +e`, `wc -c`, `seq`,
`tr -d`, `mktemp`, `wait`, `true`, `false`, `:` and `exit`.

Built-ins support only the options they implement; for example, `wc` supports
only `-c` on standard input and `tr` only `-d`. A few behaviors to know:

- `bash PATH [ARGS]` and `sh PATH [ARGS]` run a script with this same
  interpreter, in a fresh state with its own positional arguments.
- `cp -r` refuses to copy a directory into itself.
- `mv` falls back to copy and delete when moving across volumes.
- `mktemp` accepts `-d` and a template ending in at least six `X` characters,
  and never replaces an existing path.

## Not supported

The interpreter is deliberately small. It does not do glob expansion (`*`
stays literal), unquoted word splitting, brace expansion, here-documents,
functions, aliases, arrays, traps, arithmetic expansion (`$(( ))`), backtick
substitution, brace groups (`{ …; }`) or interactive job control. `>&2` is the
only descriptor duplication.

Unsupported syntax and unbalanced quotes fail with an error before anything
runs. When a task needs one of these features, the model can call an installed
interpreter explicitly through `exec`, for example `python` or `pwsh`.
