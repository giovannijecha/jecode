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
Push-Location -LiteralPath $PSScriptRoot
try {
    & cargo install --path $PSScriptRoot --locked --offline --root $InstallRoot --force
    if ($LASTEXITCODE -ne 0) {
        throw 'Cargo installation failed. The PATH was left unchanged.'
    }
} finally {
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
