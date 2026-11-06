# Preparing an npm release

The `@giovannijecha/jecode` npm package contains prebuilt native executables for
Windows x64, Linux x64, and macOS on ARM64 and x64. npm installs the package
and exposes a small Node standard-library launcher as the `jecode` command.
The launcher selects and runs the matching executable. Installing the package needs no Rust
toolchain, native compiler, third-party npm dependency, or install script. Node
must remain available when launching this npm distribution. The existing
standalone packages continue to run without Node.

## OS floors

[Platform support](PLATFORMS.md#supported-versions) lists the floors and what
has run on each. `npm/targets.cjs` holds their values. `release.mjs stage` audits
the executable and every helper embedded in it, and fails rather than record a
floor the build does not meet:

- **Linux, glibc 2.28.** `readelf` must report no version requirement above the
  floor. The runner has glibc 2.43, so the workflows link against the Debian 10
  sysroot from `tools/linux-sysroot.sh`. Weak `std` symbols stay unversioned and
  still resolve at run time on newer glibc.
- **macOS 11.0.** Each Mach-O file's recorded deployment target must not exceed
  the floor. The workflows set `MACOSX_DEPLOYMENT_TARGET`. Without it, the C
  helpers would take the build host's macOS version.
- **Windows build 17763.** A PE file records no minimum build, so the stage
  checks its subsystem version and that it imports only libraries reviewed in
  `tools/npm/windows-abi.mjs`. P/Invokes bind at run time, so
  `tools/npm/release.test.mjs` fails on one that is not reviewed there.

## Version and contents

`Cargo.toml` is the only source for the Jecode version. The release tool reads
its package version and generates `package.json`; do not edit a second version
number. A release stage records its target, version, source revision, whether
its source checkout was clean, its OS floor, and binary SHA-256. Complete packaging
requires all four targets to agree with a clean checked-out source and rejects any
artifact staged from a dirty checkout or against another floor. The package allowlist
contains the launcher, the four executables, and public documentation and license files.
Keep `AGENTS.md`, local configuration, credentials, and internal reports out of
the tarball.

The checked-in [CI workflow](../../.github/workflows/CI.yml) runs the Rust
checks, launcher tests, release build, local native package, npm lifecycle
fixture, and a native terminal check through the npm launcher on Windows x64
and Linux x64 using the existing local runners, and on hosted macOS ARM64 and x64.
Its required job names are `Rust (Windows)`, `Rust (Linux)`, `Rust (macOS)`, and
`Rust (macOS Intel)`. A queued or skipped job is not a passing release check.

## Prepare a complete tarball

1. Before selecting a version, check both `npm view @giovannijecha/jecode versions --json`
   and `npm view @giovannijecha/jecode time --json`. Removed
   versions may be absent from `versions` but remain in `time`; npm permanently
   reserves them. Choose a version that has never been published.
2. On a release branch, set the version in `Cargo.toml` and review its lockfile,
   [release notes](../../CHANGELOG.md) and user documentation together. Open a
   pull request, wait for all four CI jobs on its final revision to pass, then
   merge it to `main`. The version change must be part of that verified revision.
3. Wait for all four CI jobs on the resulting `main` commit. Run the **Prepare npm
   release** workflow on `main` while that commit is its head; the workflow has no
   revision input and builds whatever the selected branch points to. Its
   Windows and Linux jobs use this project's local runners; its macOS jobs use
   GitHub's standard hosted `macos-26` ARM64 and `macos-26-intel` x64 runners. Each
   job builds and tests the native executable, checks a local npm
   install/update/uninstall fixture and native terminal interaction through the
   launcher, then uploads a target artifact.
4. The final hosted macOS job downloads all four artifacts, verifies their
   version, revision, checkout cleanliness, OS floor and digests, assembles
   `giovannijecha-jecode-<version>.tgz`, runs the
   lifecycle fixture, and previews `npm publish <tarball> --dry-run` against
   those exact bytes. Download its `jecode-npm-tarball` artifact, including
   `manifest.json`, within seven days and inspect the digest, package inventory,
   and version before any publication decision. Confirm that `revision` in
   `manifest.json` is that commit; otherwise discard the artifacts and rerun.

The workflow does **not** publish to npm, create a Git tag or GitHub release,
or use an npm credential or OIDC write permission. The local lifecycle fixture
uses a private registry on loopback; it never contacts a provider. It checks
the native binary and npm's global install, update, and uninstall behavior.

To check one platform from a local checkout, build its release executable with
`cargo build --release --locked --offline` (on Linux, against the sysroot; on macOS,
with `MACOSX_DEPLOYMENT_TARGET=11.0`; see
[Development and testing](TESTING.md#os-floors)), then run:

```text
node tools/npm/release.mjs stage <path-to-native-executable> target/npm-stage/<target>
node tools/npm/release.mjs pack target/npm-stage target/npm-local --local
node tools/npm/lifecycle.mjs target/npm-local/giovannijecha-jecode-<version>.tgz
```

Use `win32-x64`, `linux-x64`, `darwin-arm64`, or `darwin-x64` as `<target>`. Local packaging
accepts a subset of targets and permits dirty development checkouts, but marks
its package private so it cannot be mistaken for a complete release. Complete
packaging requires a clean current checkout and artifacts staged from clean
checkouts. The stage and package output directories must not already exist.
Checkouts used for release artifacts must share the same commit.

## Publication after authorization

Publish `@giovannijecha/jecode` under the maintainer's npm user scope. Until trusted
publishing is configured, publication requires the maintainer's authenticated npm
account and 2FA, or a staged publication followed by 2FA approval. Review and publish
the exact
complete tarball produced by the preparation workflow with
`npm publish <tarball> --access public`; scoped packages need explicit public
access. Never publish a local single-platform fixture.

Trusted publishing can be configured in the package's npm settings for
`giovannijecha/jecode` and the exact future publication workflow filename. A trusted publishing job must run on a GitHub-hosted runner, with
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
[npm version reservation](https://docs.npmjs.com/policies/unpublish/),
[trusted publishing](https://docs.npmjs.com/trusted-publishers/), and
[GitHub runner architectures](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job).

## After publication

The preparation workflow does none of this. For each published version:

1. Run `npm view @giovannijecha/jecode@<version> dist.integrity` and compare it with
   `integrity` in the reviewed `manifest.json`.
2. From the previous public version, verify `npm update -g @giovannijecha/jecode`
   against the real registry, then run `jecode --version`. Use a disposable
   environment: version managers such as Volta intercept global npm commands and
   ignore `npm_config_prefix`.
3. Create the annotated tag `v<version>` with the message `Jecode <version>` on the
   `revision` recorded in `manifest.json`, and push it. That revision may be older
   than `main`.
4. Create the GitHub release `Jecode <version>` from that tag, with the user-facing
   changes as its notes, and attach `giovannijecha-jecode-<version>.tgz` and
   `manifest.json`.

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
