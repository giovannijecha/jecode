# Jecode command language

The tool named `bash` runs Jecode's own command interpreter on Windows, Linux,
and macOS. It does not start Git Bash, GNU Bash, `sh`, PowerShell, or another
installed shell. The tool name remains `bash` for conversation compatibility.
External project programs are launched through the operating system with the
parsed argument vector and must themselves be installed or present in the
project. Each tool call starts in the project directory with a fresh shell;
state such as `cd` and assignments lasts only for that call.

## Supported syntax

- Single and double quotes, backslash escapes, variables (`$NAME`, `${NAME}`,
  `${NAME-default}`, `${NAME:-default}`), `$?`, `$$`, `$!`, `$0`–`$9`, and `$@`.
- Command substitution `$(command)` with UTF-8 output and trailing newlines
  removed. It runs in a separate Jecode shell; output is limited to 1 MiB.
- Sequential commands separated by semicolons or newlines, `&&`, `||`,
  pipelines (`|`), background commands (`&`), and background subshells
  (`(commands) &`). `wait` waits for background jobs started in that call.
- Input (`<`) and output (`>`, `>>`, `2>`, `2>>`, `>&2`) redirection.
- `for NAME in WORDS; do commands; done`, `while condition; do commands; done`,
  `until condition; do commands; done`, and `if condition; then commands;
  [else commands;] fi`. A bounded `for ((NAME=START; NAME<LIMIT; NAME++)); do
  commands; done` form is also supported.

Built-in commands are `pwd`, `cd`, `echo`, `printf`, `cat`, `ls`, `mkdir`, `cp`,
`mv`, `rm`, `touch`, `test`/`[`, `sleep`, `env`, `export`, `set -e`/`set +e`,
`wc -c`, `seq`, `tr -d`, `mktemp`, `wait`, `true`, `false`, `:`, and `exit`. `bash PATH`
and `sh PATH` run a script through this same interpreter. Options are limited
to the forms implemented by these commands; for example, `wc` currently
supports only `-c` on standard input. `mktemp` accepts an optional `-d` and
a template ending in at least six `X` characters; creation is atomic and
never replaces an existing path.

The interpreter is a deliberately small command language, not a GNU Bash
implementation. It does not perform glob expansion, unquoted word splitting,
here documents, functions, aliases, arrays, traps, arithmetic expansion, or
interactive job control. `>&2` currently applies to built-in commands; use
`2>`/`2>>` to redirect an external program's error stream. Background
pipelines are not supported. Unsupported operators and malformed quotes fail with
an error; Jecode never passes a script to an installed shell as a fallback.
Use the `exec` tool when exact literal arguments or interactive terminal
control are needed.
