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
for program in bash kill stty mkfifo rm rmdir; do
    if ! command -v "$program" >/dev/null 2>&1; then
        printf '%s was not found on PATH. Install it and retry.\n' "$program" >&2
        exit 1
    fi
done

curl_executable=${JECODE_CURL:-curl}
if ! command -v "$curl_executable" >/dev/null 2>&1; then
    printf 'Selected curl was not found: %s. Check JECODE_CURL or PATH and retry.\n' "$curl_executable" >&2
    exit 1
fi
curl_details=$("$curl_executable" --version) || {
    printf 'Could not inspect curl. Install an HTTPS-enabled curl 8.3 or newer and retry.\n' >&2
    exit 1
}
curl_banner=${curl_details%%'
'*}
case $curl_banner in
    'curl '*) curl_version=${curl_banner#curl } ;;
    *) curl_version= ;;
esac
curl_version=${curl_version%% *}
curl_major=${curl_version%%.*}
curl_remaining=${curl_version#*.}
curl_minor=${curl_remaining%%.*}
curl_patch=${curl_remaining#*.}
if [ "$curl_remaining" = "$curl_version" ] || [ "$curl_patch" = "$curl_remaining" ]; then
    printf 'Could not parse curl version: %s. Install curl 8.3 or newer and retry.\n' "$curl_banner" >&2
    exit 1
fi
for number in "$curl_major" "$curl_minor" "$curl_patch"; do
    case $number in
        '' | *[!0-9]*)
            printf 'Could not parse curl version: %s. Install curl 8.3 or newer and retry.\n' "$curl_banner" >&2
            exit 1
            ;;
    esac
done
if [ "$curl_major" -lt 8 ] || { [ "$curl_major" -eq 8 ] && [ "$curl_minor" -lt 3 ]; }; then
    printf 'curl %s is too old. Install an HTTPS-enabled curl 8.3 or newer and retry.\n' "$curl_version" >&2
    exit 1
fi
curl_protocols=${curl_details#*'
Protocols:'}
if [ "$curl_protocols" = "$curl_details" ]; then
    curl_protocols=
else
    curl_protocols=${curl_protocols%%'
'*}
fi
case " $curl_protocols " in
    *' https '*) ;;
    *) printf 'curl lacks HTTPS support. Install an HTTPS-enabled curl 8.3 or newer and retry.\n' >&2; exit 1 ;;
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

printf 'Building and installing Jecode locally (offline)...\n'
(
    cd "$script_dir"
    cargo install --path "$script_dir" --locked --offline --root "$install_root" --force
)
"$binary" --version
printf '%s\n' "$owner" > "$marker"
printf 'Installed: %s\n' "$binary"
printf 'Add %s/bin to PATH, or run the executable by its full path.\n' "$install_root"
