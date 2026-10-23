[CmdletBinding()]
param(
    [string] $InstallRoot,
    [switch] $NoPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'INSTALL.ps1 supports Windows. See README.md for other platforms.'
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Cargo was not found. Install the Rust toolchain, open a new terminal, and retry.'
}
$curlExecutable = $null
if (-not [string]::IsNullOrEmpty($env:JECODE_CURL)) {
    $curlExecutable = $env:JECODE_CURL
}
if (-not $curlExecutable -and -not [string]::IsNullOrWhiteSpace($env:SystemRoot)) {
    $systemCurl = Join-Path $env:SystemRoot 'System32/curl.exe'
    if (Test-Path -LiteralPath $systemCurl -PathType Leaf) {
        $curlExecutable = $systemCurl
    }
}
if (-not $curlExecutable) {
    $curlCommand = Get-Command curl.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($curlCommand) { $curlExecutable = $curlCommand.Source }
}
if (-not $curlExecutable) {
    throw 'curl was not found. Install an HTTPS-enabled curl 8.3 or newer and retry.'
}
if (-not (Get-Command $curlExecutable -CommandType Application -ErrorAction SilentlyContinue)) {
    throw "Selected curl was not found: $curlExecutable. Check JECODE_CURL or PATH and retry."
}
$curlDetails = @(& $curlExecutable --version 2>&1)
if ($LASTEXITCODE -ne 0 -or $curlDetails.Count -eq 0) {
    throw "Could not inspect curl at $curlExecutable. Install an HTTPS-enabled curl 8.3 or newer and retry."
}
$curlBanner = [string] $curlDetails[0]
$curlVersion = [regex]::Match($curlBanner, '^curl (\d+)\.(\d+)\.(\d+)(?:\s|$)')
if (-not $curlVersion.Success -or
    [int] $curlVersion.Groups[1].Value -lt 8 -or
    ([int] $curlVersion.Groups[1].Value -eq 8 -and [int] $curlVersion.Groups[2].Value -lt 3)) {
    throw "curl at $curlExecutable must be version 8.3 or newer. Found: $curlBanner"
}
$protocolLine = @($curlDetails | Where-Object { [string] $_ -match '^Protocols:' })
$protocols = if ($protocolLine.Count) { ([string] $protocolLine[0] -replace '^Protocols:\s*', '') -split '\s+' } else { @() }
if ($protocols -cnotcontains 'https') {
    throw "curl at $curlExecutable lacks HTTPS support. Install an HTTPS-enabled curl 8.3 or newer and retry."
}
if ([string]::IsNullOrWhiteSpace($InstallRoot)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw 'LOCALAPPDATA is unavailable. Supply an absolute -InstallRoot directory.'
    }
    $InstallRoot = Join-Path $env:LOCALAPPDATA 'Jecode'
}
if (-not [IO.Path]::IsPathRooted($InstallRoot)) {
    throw 'InstallRoot must be an absolute directory path.'
}
$InstallRoot = [IO.Path]::GetFullPath($InstallRoot)
$binaryDirectory = Join-Path $InstallRoot 'bin'
$installedBinary = Join-Path $binaryDirectory 'jecode.exe'
$ownerFile = Join-Path $InstallRoot '.jecode-install'
$ownerValue = 'Jecode local Rust installation'

if (Test-Path -LiteralPath $ownerFile) {
    if ([IO.File]::ReadAllText($ownerFile).Trim() -ne $ownerValue) {
        throw "The installation marker in $InstallRoot belongs to another installation. Choose another -InstallRoot."
    }
} elseif (Test-Path -LiteralPath $installedBinary) {
    throw "An unrecognized jecode.exe already exists in $binaryDirectory. Choose another -InstallRoot."
}

Write-Host 'Building and installing Jecode locally (offline)...'
$cargoArguments = @('install', '--path', $PSScriptRoot, '--locked', '--offline', '--root', $InstallRoot, '--force')
$linkerVariable = 'CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER'
$previousLinker = [Environment]::GetEnvironmentVariable($linkerVariable, 'Process')
Push-Location -LiteralPath $PSScriptRoot
try {
    # Prefer the installed Windows toolchain used by CI. An unqualified channel
    # otherwise resolves to rustup's default host, which may require Visual C++.
    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        $toolchainFile = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'rust-toolchain.toml'))
        $channel = [regex]::Match($toolchainFile, '(?m)^\s*channel\s*=\s*"([^"]+)"').Groups[1].Value
        $windowsToolchain = "$channel-x86_64-pc-windows-gnullvm"
        $installedToolchains = @(& rustup toolchain list)
        if ($LASTEXITCODE -ne 0) { throw 'Could not list the installed Rust toolchains.' }
        if ($installedToolchains -match ('^' + [regex]::Escape($windowsToolchain) + '(\s|$)')) {
            $sysroot = & rustup run $windowsToolchain rustc --print sysroot
            if ($LASTEXITCODE -ne 0) { throw 'Could not locate the Windows Rust toolchain.' }
            $bundledLinker = Join-Path $sysroot 'lib/rustlib/x86_64-pc-windows-gnullvm/bin/rust-lld.exe'
            if (-not (Test-Path -LiteralPath $bundledLinker -PathType Leaf)) {
                throw "The bundled Windows linker is missing from $windowsToolchain. Repair this Rust toolchain and retry."
            }
            [Environment]::SetEnvironmentVariable($linkerVariable, $bundledLinker, 'Process')
            $cargoArguments = @('run', $windowsToolchain, 'cargo') + $cargoArguments + @('--target', 'x86_64-pc-windows-gnullvm')
            Write-Host "Rust toolchain: $windowsToolchain (bundled linker)"
            & rustup @cargoArguments
        } else {
            & cargo @cargoArguments
        }
    } else {
        & cargo @cargoArguments
    }
    if ($LASTEXITCODE -ne 0) {
        throw 'Cargo installation failed. The PATH was left unchanged.'
    }
} finally {
    [Environment]::SetEnvironmentVariable($linkerVariable, $previousLinker, 'Process')
    Pop-Location
}
[IO.File]::WriteAllText($ownerFile, $ownerValue + [Environment]::NewLine)
& $installedBinary --version
if ($LASTEXITCODE -ne 0) {
    throw 'The installed executable could not start. The PATH was left unchanged.'
}

function Add-DirectoryFirst {
    param([string] $ExistingPath, [string] $Directory)
    $remaining = @(foreach ($entry in ($ExistingPath -split ';')) {
        if ([string]::IsNullOrWhiteSpace($entry)) { continue }
        $normalized = [Environment]::ExpandEnvironmentVariables($entry.Trim().Trim('"')).TrimEnd('\')
        if (-not $normalized.Equals($Directory.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
            $entry
        }
    })
    (@($Directory) + @($remaining)) -join ';'
}

if ($NoPath) {
    Write-Host "Installed: $installedBinary"
    Write-Host 'PATH was left unchanged.'
} else {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $updatedUserPath = Add-DirectoryFirst -ExistingPath $userPath -Directory $binaryDirectory
    [Environment]::SetEnvironmentVariable('Path', $updatedUserPath, 'User')
    $env:Path = Add-DirectoryFirst -ExistingPath $env:Path -Directory $binaryDirectory
    $resolvedCommand = Get-Command jecode -ErrorAction Stop
    if ($resolvedCommand.CommandType -ne 'Application' -or $resolvedCommand.Source -ne $installedBinary) {
        Write-Warning "Another command overrides jecode: $($resolvedCommand.Source). Run $installedBinary directly or adjust your shell configuration."
    }
    Write-Host "Installed: $installedBinary"
    Write-Host 'This terminal is ready. Reopen other terminals to refresh their PATH.'
    Write-Host 'Open a project directory and run: jecode'
}
