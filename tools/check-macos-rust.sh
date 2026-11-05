#!/bin/sh
# Type-checks and lints the macOS Rust from another Unix host, on Apple silicon
# and Intel. It needs both Apple targets from rustup, and no Apple SDK: a stand-in
# compiler writes the empty files that build.rs embeds. The C and Objective-C
# adapters are therefore NOT compiled and nothing runs; only macOS CI covers those.
set -eu

directory=target/macos-rust-check
mkdir -p "$directory"
stub=$(cd "$directory" && pwd)/empty-output-cc
cat > "$stub" <<'EOF'
#!/bin/sh
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then : > "$2"; fi
    shift
done
EOF
chmod +x "$stub"

export CC_aarch64_apple_darwin="$stub" CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$stub"
export CC_x86_64_apple_darwin="$stub" CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER="$stub"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo clippy --locked --offline --all-targets --target "$target" -- -D warnings
done
