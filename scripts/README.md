# Repository checks and release tooling

These scripts belong to the reproducible source repository. They build and
verify the installed runtime, so keep them available to CI and source checkouts.
They are excluded from the npm package by its explicit file whitelist.
The optional development environments live under [`dev/`](../dev/README.md).

Run these commands from the repository root after `npm ci --ignore-scripts`:

| Command | Purpose |
| --- | --- |
| `npm run build:release` | Remove the repository's generated `dist/`, compile `src/`, and verify that the output contains only JavaScript. |
| `npm run pack:release` | Clean-build and create the npm tarball with lifecycle scripts disabled. |
| `npm run check:source-tree` | Verify that `dist/` is ignored and untracked. |
| `npm run check:package` | Clean-build and inspect npm's dry-run manifest, runtime dependencies, packaged README references, and release settings. |
| `npm run check:install` | Clean-build, pack, install into a temporary global prefix, and verify the installed executable's version. |
| `npm run check:release-tag -- <tag>` | Require the exact package version and select `latest` for stable versions or `next` for prereleases; substitute the actual tag. |
| `npm run check` | Run source-tree, type, coverage, package, and installation gates. |

The package and installation checks intentionally remain independently runnable
and each performs its own clean build. `build:release` also accepts `--quiet`.
Generated `dist/` is never source code and must remain untracked. Installation
and packaging lifecycle hooks must not implicitly compile the runtime.

`release-policy.ts` holds the shared release-channel and README rules.
`check-release-tag.ts` reads `package.json` from the working directory and exports
`channel` to `GITHUB_OUTPUT` when GitHub Actions supplies that path. Its integration
tests use isolated fixture manifests for stable and prerelease versions, so a
candidate can be checked without changing the repository's current version.

The workflows call the npm commands above. A file move must update the matching
script path, TypeScript configuration, tests, and the explicit command guard in
`check-package.ts`. The actual publishing procedure belongs in the
[release guide](../docs/RELEASING.md).
