# Jecode

Jecode is a terminal coding agent. It runs as a native Rust executable and lets
you work with an AI model on files and commands in your project.

The first npm release is being prepared and is not yet published. Once it is
available, install Jecode with Node.js 22 or later and npm:

```sh
npm install -g jecode
jecode --version
```

The npm package supports Windows x64, Linux x64 with glibc 2.39 or newer, and
macOS on Apple silicon (verified on macOS 15). Earlier macOS versions remain
unverified. It includes the platform executables; it does not build code or fetch
a binary when you first run it. Node.js starts the npm command. A standalone
build of Jecode does not need Node.js.

Update the npm installation with:

```sh
npm update -g jecode
```

Remove it with:

```sh
npm uninstall -g jecode
```

Removing the package keeps your settings, accounts, conversations, and
checkpoints in `~/.jecode/` or your `JECODE_HOME` directory. If you previously
installed Jecode from source, check which `jecode` command appears first on
`PATH`; npm does not remove that separate installation.

See the [installation guide](https://github.com/giovannijecha/jecode/blob/main/docs/usage/INSTALLATION.md)
for source installation and troubleshooting, and the [command-line guide](https://github.com/giovannijecha/jecode/blob/main/docs/usage/CLI.md)
for usage.
