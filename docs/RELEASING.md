# Releasing

Versions use SemVer. Alpha versions identify an incomplete, experimental product;
they do not imply independent security review or support for untested platforms.
The current source candidate is `0.1.0-alpha.1`.

From a clean, reviewed checkout:

```text
cargo run --locked --offline --bin jecode-check -- release-check v0.1.0-alpha.1
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
distribution tag. No native npm installer is included in this candidate.
