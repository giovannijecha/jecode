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
  `"$@"` expands to one word per positional argument.
- Command substitution `$(command)` with UTF-8 output and trailing newlines
  removed. It runs in a separate Jecode shell; output is limited to 1 MiB. Its
  output is used whatever its exit status, and an assignment-only command such
  as `x=$(command)` sets `$?` to that status. It reads the active input stream
  and writes diagnostics to the active error stream.
- Sequential commands separated by semicolons or newlines, `&&`, `||`,
  pipelines (`|`), and background commands (`&`). A newline or comment after
  `&&`, `||` or `|` continues the command on the next line. Pipeline stages run
  concurrently, each in its own subshell. A background command, pipeline or
  subshell runs in a child shell that expands it there; `wait` waits for
  background jobs started in that call. A producer writing to a closed
  downstream pipe stops, including inside compound commands; commands after
  the pipeline still run.
- `NAME=value command` sets `NAME` for that command only, including built-ins,
  pipeline stages and background jobs.
- Input (`<`) and output (`>`, `>>`, `2>`, `2>>`, `>&2`) redirection. A
  descriptor number counts only when written directly before `>`, so
  `echo 1 > file` writes `1`. `/dev/null` discards output or supplies empty
  input on every platform. A redirection without a command, such as
  `> file`, creates or truncates the file.
- `for NAME in WORDS; do commands; done`, `while condition; do commands; done`,
  `until condition; do commands; done`, `if condition; then commands; [elif
  condition; then commands;] [else commands;] fi`, and subshells
  `(commands)`. A bounded `for ((NAME=START; NAME<LIMIT; NAME++)); do
  commands; done` form is also supported. These compound commands nest, take
  redirections after their closing word (`done > file`), and can appear in
  `&&`/`||` lists and pipelines.
- `{}` and other braces without a comma or `..` are ordinary word characters,
  as in `find . -exec cmd {} \;` or `HEAD@{1}`.

Built-in commands are `pwd`, `cd`, `echo`, `printf`, `cat`, `ls`, `mkdir`, `cp`,
`mv`, `rm`, `touch`, `test`/`[`, `sleep`, `env`, `export`, `set -e`/`set +e`,
`wc -c`, `seq`, `tr -d`, `mktemp`, `wait`, `true`, `false`, `:`, and `exit`.
`bash PATH [ARGS]` and `sh PATH [ARGS]` run a script through this same
interpreter in a fresh shell with its own positional arguments; its variables,
directory and `exit` do not affect the caller. Options are limited to the forms
implemented by these commands; for example, `wc` currently supports only `-c`
on standard input. `cp -r` refuses to copy a directory into itself, and `mv`
falls back to copying and removing when a rename crosses volumes. `mktemp`
accepts an optional `-d` and a template ending in at least six `X` characters;
creation is atomic and never replaces an existing path.

On Windows, variable names are case-insensitive like the environment, so
`$PATH` reads `Path` and `export PATH=...` updates it.

The interpreter is a deliberately small command language, not a GNU Bash
implementation. It does not perform glob expansion, unquoted word splitting,
brace expansion, here documents, functions, aliases, arrays, traps, arithmetic
expansion, brace groups (`{ commands; }`), or interactive job control. `>&2` is
the only descriptor duplication. Unsupported operators and malformed quotes fail
with an error before any command runs; Jecode never passes a script to an
installed shell as a fallback. Use the `exec` tool when exact literal arguments
or interactive terminal control are needed.
