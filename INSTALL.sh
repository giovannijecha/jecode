#!/bin/sh
set -eu

if [ "$#" -gt 1 ]; then
    printf 'Usage: sh INSTALL.sh [absolute install root]\n' >&2
    exit 2
fi

case $(uname -s) in
    Linux | Darwin) ;;
    *) printf 'INSTALL.sh supports Linux and macOS. Use INSTALL.ps1 on Windows.\n' >&2; exit 1 ;;
esac

if ! command -v cargo >/dev/null 2>&1; then
    printf 'Cargo was not found. Install Rust 1.95.0 and retry.\n' >&2
    exit 1
fi
for program in bash curl kill stty mkfifo rm rmdir; do
    if ! command -v "$program" >/dev/null 2>&1; then
        printf '%s was not found on PATH. Install it and retry.\n' "$program" >&2
        exit 1
    fi
done

if [ "$#" -eq 1 ]; then
    install_root=$1
else
    if [ -z "${HOME:-}" ]; then
        printf 'HOME is unavailable. Supply an absolute install root.\n' >&2
        exit 1
    fi
    install_root=${XDG_DATA_HOME:-"$HOME/.local/share"}/jecode
fi
case $install_root in
    /*) ;;
    *) printf 'Install root must be an absolute directory path.\n' >&2; exit 2 ;;
esac

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
binary=$install_root/bin/jecode
marker=$install_root/.jecode-install
owner='Jecode local Rust installation'

if [ -L "$marker" ]; then
    printf 'Installation marker is a symbolic link: %s\n' "$marker" >&2
    exit 1
fi
if [ -e "$marker" ]; then
    if [ ! -f "$marker" ] || [ "$(cat "$marker")" != "$owner" ]; then
        printf 'Installation marker belongs to another installation: %s\n' "$marker" >&2
        exit 1
    fi
elif [ -e "$binary" ] || [ -L "$binary" ]; then
    printf 'An unrecognized jecode executable exists: %s\n' "$binary" >&2
    exit 1
fi

printf 'Building and installing Jecode locally (offline)...\n'
(
    cd "$script_dir"
    cargo install --path "$script_dir" --locked --offline --root "$install_root" --force
)
"$binary" --version
printf '%s\n' "$owner" > "$marker"
printf 'Installed: %s\n' "$binary"
printf 'Add %s/bin to PATH, or run the executable by its full path.\n' "$install_root"
