#!/bin/sh
set -eu
# Developer-only build step; end users receive the prebuilt executable.
script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
cd "$script_dir"
cargo build --release --locked --offline
target_root=${CARGO_TARGET_DIR:-"$script_dir/target"}
architecture=$(rustc -vV | sed -n 's/^host: \([^-]*\).*/\1/p')
case $(uname -s) in
    Linux) platform=linux ;;
    Darwin) platform=macos ;;
    *) printf 'Use PACKAGE.ps1 on Windows.\n' >&2; exit 2 ;;
esac
package=$target_root/package/jecode-$platform-$architecture
if [ -e "$package" ] || [ -L "$package" ]; then
    printf 'Package destination already exists: %s\n' "$package" >&2
    exit 1
fi

set --
while IFS= read -r relative || [ -n "$relative" ]; do
    case "/$relative/" in
        *//*|*/./*|*/../*|*\\*|*:*)
            printf 'Invalid package documentation path: %s\n' "$relative" >&2
            exit 1 ;;
    esac
    entry=$script_dir/docs/$relative
    if [ ! -f "$entry" ]; then
        printf 'Missing package documentation file: %s\n' "$relative" >&2
        exit 1
    fi
    while [ "$entry" != "$script_dir" ]; do
        if [ -L "$entry" ]; then
            printf 'Linked package documentation path: %s\n' "$relative" >&2
            exit 1
        fi
        entry=${entry%/*}
    done
    set -- "$@" "$relative"
done < "$script_dir/tools/package-docs.txt"
if [ "$#" -eq 0 ]; then
    printf 'Package documentation list is empty.\n' >&2
    exit 1
fi

mkdir -p "$(dirname "$package")"
mkdir "$package"
cp "$target_root/release/jecode" "$package/jecode"
cp "$script_dir/INSTALL.sh" "$script_dir/LICENSE" "$script_dir/README.md" \
    "$script_dir/CHANGELOG.md" "$script_dir/SECURITY.md" "$script_dir/CONTRIBUTING.md" \
    "$script_dir/CODE_OF_CONDUCT.md" "$package/"
for relative in "$@"; do
    mkdir -p "$package/docs/$(dirname "$relative")"
    cp "$script_dir/docs/$relative" "$package/docs/$relative"
done
chmod 755 "$package/jecode"
if command -v sha256sum >/dev/null 2>&1; then
    digest=$(sha256sum "$package/jecode")
else
    digest=$(shasum -a 256 "$package/jecode")
fi
printf '%s  jecode\n' "${digest%% *}" > "$package/SHA256SUMS"
printf 'Prepared local package: %s\n' "$package"
