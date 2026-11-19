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
    if (Test-Path -LiteralPath $package) { throw "Package destination already exists: $package" }

    $docsRoot = Join-Path $PSScriptRoot 'docs'
    $documents = @(Get-Content -LiteralPath (Join-Path $PSScriptRoot 'tools/package-docs.txt'))
    if ($documents.Count -eq 0) { throw 'Package documentation list is empty.' }
    foreach ($relative in $documents) {
        if ($relative -match '(^|/)\.{0,2}(/|$)|[\\:]') { throw "Invalid package documentation path: $relative" }
        $entry = Join-Path $docsRoot $relative
        if (-not (Test-Path -LiteralPath $entry -PathType Leaf)) { throw "Missing package documentation file: $relative" }
        while ($entry -ne $PSScriptRoot) {
            if ((Get-Item -LiteralPath $entry -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Linked package documentation path: $relative"
            }
            $entry = Split-Path -Parent $entry
        }
    }

    New-Item -ItemType Directory -Path (Split-Path -Parent $package) -Force | Out-Null
    New-Item -ItemType Directory -Path $package -ErrorAction Stop | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $package 'jecode.exe') -Force
    foreach ($name in @('INSTALL.ps1', 'LICENSE', 'README.md', 'CHANGELOG.md', 'SECURITY.md', 'CONTRIBUTING.md', 'CODE_OF_CONDUCT.md')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $package $name) -Force
    }
    foreach ($relative in $documents) {
        $destination = Join-Path (Join-Path $package 'docs') $relative
        [IO.Directory]::CreateDirectory((Split-Path -Parent $destination)) | Out-Null
        Copy-Item -LiteralPath (Join-Path $docsRoot $relative) -Destination $destination
    }
    $digest = (Get-FileHash -LiteralPath (Join-Path $package 'jecode.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $package 'SHA256SUMS'), $digest + '  jecode.exe' + "`n")
    Write-Host "Prepared local package: $package"
} finally { Pop-Location }
