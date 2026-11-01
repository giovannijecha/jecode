# Preparing an npm release

The `@giovannijecha/jecode` npm package contains prebuilt native executables for
Windows x64, Linux x64 (GNU libc 2.39 or newer), and macOS ARM64. npm installs the package
and exposes a small Node standard-library launcher as the `jecode` command.
The launcher selects and runs the matching executable. Installing the package needs no Rust
toolchain, native compiler, third-party npm dependency, or install script. Node
must remain available when launching this npm distribution. The existing
standalone packages continue to run without Node.

Inspection of this release build's ELF version requirements found GLIBC_2.39
in the main executable and embedded Rust shell helper, and at most GLIBC_2.34
in the six embedded C helpers. The Linux runner uses GNU libc 2.43. The 2.39
threshold comes from the binaries; runtime behavior on older hosts has not
been tested. The Linux stage checks the main executable and all seven embedded
ELF helpers with `readelf` and rejects any version requirement above 2.39.

The macOS ARM64 build and terminal fixture run on macOS 15. The deployment
targets of the executable and embedded native helpers have not been inspected,
and earlier macOS versions have not been tested.

macOS Intel is not in the npm package. Cross compilation from macOS ARM64 is
possible in the build script, but no native macOS Intel release check has been
recorded. Add a native build and lifecycle test on a standard
`macos-15-intel` runner before shipping that target.

## Version and contents

`Cargo.toml` is the only source for the Jecode version. The release tool reads
its package version and generates `package.json`; do not edit a second version
number. A release stage records its target, version, source revision, whether
its source checkout was clean, and binary SHA-256. Complete packaging requires
all three targets to agree with a clean checked-out source and rejects any
artifact staged from a dirty checkout. The package allowlist contains the
launcher, the three executables, and public documentation and license files.
Keep `AGENTS.md`, local configuration, credentials, and internal reports out of
the tarball.

The checked-in [CI workflow](../../.github/workflows/CI.yml) runs the Rust
checks, launcher tests, release build, local native package, npm lifecycle
fixture, and a native terminal check through the npm launcher on Windows x64
and Linux x64 using the existing local runners, and on hosted macOS ARM64. Its
required job names remain `Rust (Windows)`,
`Rust (Linux)`, and `Rust (macOS)`. A queued or skipped job is not a passing
release check.

## Prepare a complete tarball

1. Merge the intended revision to `main` after its three CI jobs pass. Set the
   release version in `Cargo.toml` and review the corresponding lockfile and
   user documentation in that revision.
2. Run the **Prepare npm release** workflow manually for that revision. Its
   Windows and Linux jobs use this project's local runners; its macOS job uses
   GitHub's standard hosted `macos-15` ARM64 runner. Each job builds and tests
   the native executable, checks a local npm install/update/uninstall fixture
   and native terminal interaction through the launcher, then uploads a target
   artifact.
3. The final hosted macOS job downloads all three artifacts, verifies their
   version, revision, checkout cleanliness, and digests, assembles
   `giovannijecha-jecode-<version>.tgz`, runs the
   lifecycle fixture, and previews `npm publish <tarball> --dry-run` against
   those exact bytes. Download its `jecode-npm-tarball` artifact, including
   `manifest.json`, within seven days and inspect the digest, package inventory,
   and version before any publication decision.

The workflow does **not** publish to npm, create a Git tag or GitHub release,
or use an npm credential or OIDC write permission. The local lifecycle fixture
uses a private registry on loopback; it never contacts a provider. It checks
the native binary and npm's global install, update, and uninstall behavior.
Once two public versions exist, also verify
`npm update -g @giovannijecha/jecode` against the real registry from the earlier
version.

To check one platform from a local checkout, build its release executable with
`cargo build --release --locked --offline`, then run:

```text
node tools/npm/release.mjs stage <path-to-native-executable> target/npm-stage/<target>
node tools/npm/release.mjs pack target/npm-stage target/npm-local --local
node tools/npm/lifecycle.mjs target/npm-local/giovannijecha-jecode-<version>.tgz
```

Use `win32-x64`, `linux-x64`, or `darwin-arm64` as `<target>`. Local packaging
accepts a subset of targets and permits dirty development checkouts, but marks
its package private so it cannot be mistaken for a complete release. Complete
packaging requires a clean current checkout and artifacts staged from clean
checkouts. The stage and package output directories must not already exist.
Checkouts used for release artifacts must share the same commit.

## Publication after authorization

Publish `@giovannijecha/jecode` under the maintainer's npm user scope. The first
publication requires the maintainer's authenticated npm account and 2FA, or a
staged publication followed by 2FA approval. Review and publish the exact
complete tarball produced by the preparation workflow with
`npm publish <tarball> --access public`; scoped packages need explicit public
access. Never publish a local single-platform fixture.

After the first package exists, trusted publishing can be configured in its npm
settings for `giovannijecha/jecode` and the exact future publication workflow
filename. A trusted publishing job must run on a GitHub-hosted runner, with
`id-token: write`, Node 22.14.0 or newer, and npm 11.5.1 or newer. The
publisher configuration must explicitly allow direct `npm publish`; its first
successful use must happen within two days. Keep Windows and Linux build and
test jobs on the local runners; a hosted macOS job can publish an already
verified tarball after all target jobs pass. Do not add an npm token to the
repository. The package's `repository.url` must exactly identify this GitHub
repository. Configure these permissions only when actual publication is
authorized and the workflow is ready.

References: [npm package binaries](https://docs.npmjs.com/cli/v11/configuring-npm/package-json/#bin),
[global installation](https://docs.npmjs.com/downloading-and-installing-packages-globally/),
[global update](https://docs.npmjs.com/updating-packages-downloaded-from-the-registry/),
[global uninstall](https://docs.npmjs.com/uninstalling-packages-and-dependencies/),
[first public publication](https://docs.npmjs.com/creating-and-publishing-scoped-public-packages/),
[trusted publishing](https://docs.npmjs.com/trusted-publishers/), and
[GitHub runner architectures](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job).

## User lifecycle

The npm commands are:

```text
npm install -g @giovannijecha/jecode
jecode --version
npm update -g @giovannijecha/jecode
npm uninstall -g @giovannijecha/jecode
```

npm uninstall removes the npm package and command shim. It does not remove
Jecode's configuration, credentials, conversations, or checkpoints under
`~/.jecode/`. The separate standalone installer may have added another
`jecode` earlier on `PATH`; check `Get-Command jecode` on Windows or
`command -v jecode` on Unix before and after migration.
