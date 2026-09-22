# Releasing

Versions use SemVer. Alpha versions identify an incomplete, experimental product;
they do not imply independent security review or support for untested platforms.
The current source candidate is `0.1.0-alpha.2`.

From a clean, reviewed checkout:

```text
cargo run --locked --offline --bin jecode-check -- release-check v0.1.0-alpha.2
```

The command requires a matching version, clean source, an empty external dependency
graph, public package inventory, formatting, Clippy, offline tests and a release
build. It does not publish.

Before publishing a tag or release:

1. Pass Windows and Linux CI on the exact committed source.
2. Build native artifacts, inspect runtime dependencies and test startup outside
   the build toolchain's PATH.
3. Exercise sign-in, a real task, cancellation and resume using the artifacts.
4. Package each supported target with its license and SHA-256 checksum.
5. Publish an explicitly marked prerelease with exact capabilities and limitations.

Keep local notes, user data, credentials, archives and agent instructions out of
the public source and artifacts. Include tests and the owned check runner so
verification remains reproducible.

The npm package name is `@giovannijecha/jecode`. A registry version cannot be reused
after publication or unpublishing. Native npm distribution must preserve that
constraint and verify its installer and platform artifacts before changing a
distribution tag. The first npm alpha supports Windows x64. Linux x64 uses the
native release archive. No JavaScript launcher or install script is required.

After building `target/release/jecode.exe` from the verified commit:

```text
cargo run --locked --offline --bin jecode-check -- package-windows target/npm-candidate
npm pack ./target/npm-candidate --ignore-scripts
```

The owned packager checks the native PE architecture and version, then copies
only the executable, README, license and generated manifest to a new directory.
Inspect the four-file tarball inventory, install it into an isolated npm prefix,
test `jecode --version` and startup, and verify replacement of the retired package.
The app must also launch with Node and the build toolchain absent from PATH.
Publish the tested tarball with `--tag next --access public`. Only point `latest`
at it after verifying a registry download; keep the alpha label explicit.
