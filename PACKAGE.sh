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
mkdir -p "$package"
cp "$target_root/release/jecode" "$package/jecode"
cp "$script_dir/INSTALL.sh" "$script_dir/LICENSE" "$script_dir/README.md" \
    "$script_dir/SECURITY.md" "$script_dir/CONTRIBUTING.md" "$package/"
mkdir -p "$package/docs"
# Without -L, find neither follows nor matches symbolic links.
find "$script_dir/docs" -type f ! -name 'HARNESS*' ! -name AGENTS.md ! -name CLAUDE.md | while IFS= read -r document; do
    relative=${document#"$script_dir/docs/"}
    mkdir -p "$package/docs/$(dirname "$relative")"
    cp "$document" "$package/docs/$relative"
done
chmod 755 "$package/jecode"
if command -v sha256sum >/dev/null 2>&1; then
    digest=$(sha256sum "$package/jecode")
else
    digest=$(shasum -a 256 "$package/jecode")
fi
printf '%s  jecode\n' "${digest%% *}" > "$package/SHA256SUMS"
printf 'Prepared local package: %s\n' "$package"
