#!/bin/sh
# Prepares a Debian 10 sysroot (glibc 2.28) and a compiler wrapper that links
# against it, so Linux builds run on every distribution with glibc 2.28 or newer.
#
#   sh tools/linux-sysroot.sh <directory>
#
# Prints the two environment assignments Cargo and build.rs read. The packages
# are pinned by SHA-256; an existing valid sysroot is reused.
set -eu

if [ "$#" -ne 1 ]; then
    printf 'Usage: sh tools/linux-sysroot.sh <directory>\n' >&2
    exit 2
fi
mkdir -p "$1"
root=$(cd "$1" && pwd)
mirror=https://archive.debian.org/debian/pool/main
stamp=$root/.complete

fetch() {
    file=$root/.packages/$(basename "$1")
    if [ ! -f "$file" ] || ! printf '%s  %s\n' "$2" "$file" | sha256sum -c - >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
            --retry 3 --output "$file" "$mirror/$1"
        printf '%s  %s\n' "$2" "$file" | sha256sum -c - >/dev/null
    fi
    dpkg-deb --extract "$file" "$root"
}

if [ ! -f "$stamp" ]; then
    mkdir -p "$root/.packages"
    fetch g/glibc/libc6_2.28-10+deb10u1_amd64.deb 0ee0f0bfcd7b59ffda1dbc76ef58fa79512913204f2a336206467bfbf45bc69f
    fetch g/glibc/libc6-dev_2.28-10+deb10u1_amd64.deb 76b8413920d91a9a4289f99839933d0d30095e0ec12b05bb4f1a7f27e5ef4a55
    fetch l/linux/linux-libc-dev_4.19.249-2_amd64.deb c512f30f5abe413464b79ed23b54958df8cc104446c3ae93d83d28e23728a571
    fetch g/gcc-8/libgcc1_8.3.0-6_amd64.deb b1bb7611f3372732889d502cb1d09fe572b5fbb5288a4a8b1ed0363fecc3555a
    fetch g/gcc-8/libgcc-8-dev_8.3.0-6_amd64.deb a52d5216858c7185bc245e52ad95a6ad515d087bc25e8556df06c5626b41a837

    # Debian's development symlinks are absolute and would resolve to the host's
    # newer libraries; keep them inside the sysroot.
    find "$root" -type l | while IFS= read -r link; do
        value=$(readlink "$link")
        case $value in
            /*) ln -sfn "$root$value" "$link" ;;
        esac
    done
    rm -rf "$root/.packages" "$root/usr/share"

    # A distribution GCC searches its own library directories ahead of --sysroot,
    # so name the sysroot's start files, libgcc and libc first. The host's would
    # reference glibc symbols newer than the sysroot provides.
    {
        printf '#!/bin/sh\n'
        printf 'root="%s"\n' "$root"
        printf 'exec cc --sysroot="$root" -B"$root/usr/lib/gcc/x86_64-linux-gnu/8/" \\\n'
        printf '    -B"$root/usr/lib/x86_64-linux-gnu/" -L"$root/usr/lib/x86_64-linux-gnu" \\\n'
        printf '    -L"$root/lib/x86_64-linux-gnu" "$@"\n'
    } > "$root/cc"
    chmod +x "$root/cc"
    : > "$stamp"
fi

printf 'CC_x86_64_unknown_linux_gnu=%s/cc\n' "$root"
printf 'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=%s/cc\n' "$root"
