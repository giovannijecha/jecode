[CmdletBinding()]
param(
    [string] $InstallRoot,
    [string] $BinaryPath,
    [switch] $NoPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'INSTALL.ps1 supports Windows. See README.md for other platforms.'
}
if ([string]::IsNullOrWhiteSpace($BinaryPath)) {
    $BinaryPath = Join-Path $PSScriptRoot 'jecode.exe'
}
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw 'This installer needs the prebuilt jecode.exe supplied in the platform package. Building from source is a separate developer operation.'
}
$BinaryPath = (Resolve-Path -LiteralPath $BinaryPath).Path
& $BinaryPath --version
if ($LASTEXITCODE -ne 0) { throw 'The supplied executable could not start.' }
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

[IO.Directory]::CreateDirectory($binaryDirectory) | Out-Null
if (-not [IO.File]::Exists($ownerFile)) {
    $markerStage = Join-Path $InstallRoot ('.jecode-marker-' + [Guid]::NewGuid().ToString('N'))
    $markerOwned = $false
    try {
        $markerStream = [IO.File]::Open($markerStage, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $markerOwned = $true
        try {
            $markerBytes = [Text.Encoding]::UTF8.GetBytes($ownerValue + [Environment]::NewLine)
            $markerStream.Write($markerBytes, 0, $markerBytes.Length)
            $markerStream.Flush($true)
        } finally { $markerStream.Dispose() }
        [IO.File]::Move($markerStage, $ownerFile)
    } finally {
        if ($markerOwned -and [IO.File]::Exists($markerStage)) { [IO.File]::Delete($markerStage) }
    }
}
if (-not $BinaryPath.Equals($installedBinary, [StringComparison]::OrdinalIgnoreCase)) {
    $stagedBinary = Join-Path $binaryDirectory ('jecode-' + [Guid]::NewGuid().ToString('N') + '.exe')
    $stageOwned = $false
    try {
        $stageStream = [IO.File]::Open($stagedBinary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $stageOwned = $true
        try {
            $sourceStream = [IO.File]::OpenRead($BinaryPath)
            try { $sourceStream.CopyTo($stageStream) } finally { $sourceStream.Dispose() }
            $stageStream.Flush($true)
        } finally { $stageStream.Dispose() }
        & $stagedBinary --version
        if ($LASTEXITCODE -ne 0) { throw 'The staged executable could not start.' }
        if (Test-Path -LiteralPath $installedBinary) {
            [IO.File]::Replace($stagedBinary, $installedBinary, [NullString]::Value)
        } else {
            [IO.File]::Move($stagedBinary, $installedBinary)
        }
    } finally {
        if ($stageOwned -and [IO.File]::Exists($stagedBinary)) { [IO.File]::Delete($stagedBinary) }
    }
}
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
