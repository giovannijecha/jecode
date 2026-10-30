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

source_binary=$script_dir/jecode
if [ ! -f "$source_binary" ] || [ ! -x "$source_binary" ]; then
    printf 'This installer needs the prebuilt jecode supplied in the platform package. Building source is a separate developer operation.\n' >&2
    exit 1
fi
"$source_binary" --version
mkdir -p "$install_root/bin"
staged=$(mktemp "$install_root/bin/.jecode-install.XXXXXXXXXX")
marker_stage=
trap 'rm -f -- "$staged"; if [ -n "$marker_stage" ]; then rm -f -- "$marker_stage"; fi' EXIT HUP INT TERM
if [ ! -e "$marker" ]; then
    marker_stage=$(mktemp "$install_root/.jecode-marker.XXXXXXXXXX")
    printf '%s\n' "$owner" > "$marker_stage"
    ln "$marker_stage" "$marker"
    rm -f -- "$marker_stage"
    marker_stage=
fi
cp "$source_binary" "$staged"
chmod 755 "$staged"
"$staged" --version
mv -f "$staged" "$binary"
"$binary" --version
printf 'Installed: %s\n' "$binary"
printf 'Add %s/bin to PATH, or run the executable by its full path.\n' "$install_root"
