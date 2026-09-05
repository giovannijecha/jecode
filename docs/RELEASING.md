# Release Jecode

This is the maintainer runbook for publishing Jecode. Releases originate from a
reviewed commit on `main`; the tag-triggered GitHub workflow builds and publishes
the npm artifact through trusted publishing.

For the 1.0 cycle, follow the
[release-candidate requirements](COMPATIBILITY.md#release-candidate-gate),
including the soak before stable promotion.

## Prepare a stable release

1. Create a release branch from current `main`.
2. Update the version in `package.json` and both root-package entries in
   `package-lock.json`.
3. Promote the accumulated changelog entries to the dated release and update
   version examples that users will submit in bug reports.
4. Remove prerelease installation instructions when no active candidate exists.
5. Run `npm run check`, open a pull request, and merge only after review and
   every CI job pass.
6. Tag the exact merge commit and push the immutable tag. In PowerShell, derive
   it from the reviewed manifest rather than typing the version twice:

```powershell
$releaseVersion = node -p "require('./package.json').version"
git tag --annotate "v$releaseVersion" --message "Jecode $releaseVersion"
git push origin "v$releaseVersion"
```

7. Verify the Publish workflow, the exact registry version, provenance, package
   contents, and `latest` dist-tag before announcing the release.

## Manage the prerelease channel

A version containing a hyphen, such as `1.0.0-rc.1`, is published to `next`.
Stable versions are published to `latest`. README instructions may advertise
`next` only while a current release-candidate cycle is active.

[Trusted publishing](https://docs.npmjs.com/trusted-publishers/) authenticates
`npm publish` but does not authenticate `npm dist-tag`. After a stable release
ends a candidate cycle, an npm maintainer removes the stale tag interactively;
never add a registry token to the repository or publish workflow.

```console
npm login
```

```console
npm dist-tag rm @giovannijecha/jecode next
```

```console
npm logout
```

Confirm the resulting public state:

```console
npm view @giovannijecha/jecode dist-tags --json
```

Do not remove `next` while an announced prerelease remains active.

## Recover from a failed publish

Do not move a release tag or reuse a version already accepted by npm. Diagnose
the workflow, fix the cause through the normal pull-request path, increment the
version when necessary, and publish from a new immutable tag.
