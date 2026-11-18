# Native readiness for the benchmark lab

Use the readiness launcher after staging a pilot in the dedicated Windows/WSL
lab. The lab workspace holds the local staging tools and retained campaign data.
The reviewed checkout supplies the readiness scripts.

```powershell
$lab = 'PATH_TO_LOCAL_LAB_WORKSPACE'
./tools/benchmark-pilot-ready.ps1 -Action Probe -LabWorkspace $lab -RunId $id
```

`Probe` accepts staged and historical pilot checkpoints for offline diagnosis.
It uses the staged image digest, the image's configured user, and the staged
read-only agent, ACP and OS runtime binds on the quota daemon. It checks the
effective UID, runtime digests, native version and ACP v1 initialization.
The Codex profile checks its mounted code-mode host with help and closed stdin.
The Jecode profiles send a known SHA-256 vector and a random-byte request to
the exact crypto helper embedded in the staged build.

For the Jecode and Codex profiles, the staged ACP Python loads the OS OpenSSL
libraries and initializes a TLS context with OpenSSL's default verification
paths. The check expects a default CA bundle and requires parsed CA certificates
in the store because the OpenSSL call can return success with an empty store.
It records the compiled paths, the bundle digest when available,
and the number of loaded CA certificates. It rejects certificate environment
overrides from the staged configuration or image. The Claude profile runs the
native version and ACP checks for its own runtime.

Use `Run` as the launch entry point for a fresh, reviewed pilot. It runs the
same offline probe and invokes the local pilot launcher only after a pass.
The local pilot selects the task network and starts Harbor after the campaign's
account and quota checks.

```powershell
./tools/benchmark-pilot-ready.ps1 -Action Run -LabWorkspace $lab -RunId $id
```

The readiness sandbox uses Docker's `none` network, local images, and a scratch
home. Receipts are written to a new directory per check under
`/var/lib/jecode-benchmark/native-readiness/` inside WSL. They include the
staging and runtime digests, effective UID and GID, individual outcomes, and the
probe container's retirement status. `public_https: "not_run"`,
`account_access: false`, and `model_requests: 0` describe the offline phase.
Record any authorized public HTTPS diagnostic separately from offline readiness
and any later model-access check.

For a regression check, probe the original React staging, a disposable copy
with the OS libraries and only the system CA path, and the corrected staging
with the bundle at both paths. The first should report unavailable crypto and
TLS, the second should pass crypto and load zero CA certificates, and the third
should pass both. Preserve the original campaign and give any changed build or
protocol a distinct campaign before running model trials.
