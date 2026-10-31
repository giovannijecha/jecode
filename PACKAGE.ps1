[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Developer-only build step. The generated package needs no runtime compiler.
Push-Location -LiteralPath $PSScriptRoot
try {
    & cargo build --release --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'The release build failed.' }
    $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot 'target' }
    $targetRoot = [IO.Path]::GetFullPath($targetRoot)
    $binary = Join-Path $targetRoot 'release/jecode.exe'
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'The native Windows release executable was not produced.' }
    $hostInfo = @(& rustc -vV)
    if ($LASTEXITCODE -ne 0) { throw 'Could not identify the built architecture.' }
    $hostTriple = ([string]($hostInfo | Where-Object { $_ -like 'host: *' })).Substring(6)
    $package = Join-Path $targetRoot ('package/jecode-windows-' + $hostTriple.Split('-')[0])
    New-Item -ItemType Directory -Path $package -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $package 'jecode.exe') -Force
    foreach ($name in @('INSTALL.ps1', 'LICENSE', 'README.md', 'SECURITY.md', 'CONTRIBUTING.md')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $package $name) -Force
    }
    $docsRoot = Join-Path $PSScriptRoot 'docs'
    foreach ($document in Get-ChildItem -LiteralPath $docsRoot -Recurse -File) {
        if ($document.Name -like 'HARNESS*' -or $document.Name -in @('AGENTS.md', 'CLAUDE.md') -or
            ($document.Attributes -band [IO.FileAttributes]::ReparsePoint)) { continue }
        $relative = $document.FullName.Substring($docsRoot.Length).TrimStart('\', '/')
        $destination = Join-Path (Join-Path $package 'docs') $relative
        [IO.Directory]::CreateDirectory((Split-Path -Parent $destination)) | Out-Null
        Copy-Item -LiteralPath $document.FullName -Destination $destination -Force
    }
    $digest = (Get-FileHash -LiteralPath (Join-Path $package 'jecode.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $package 'SHA256SUMS'), $digest + '  jecode.exe' + "`n")
    Write-Host "Prepared local package: $package"
} finally { Pop-Location }
