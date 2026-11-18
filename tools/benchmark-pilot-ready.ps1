# Probe the exact staged pilot runtime before handing off to the local launcher.
param(
  [ValidateSet('Probe','Run')][string]$Action = 'Probe',
  [Parameter(Mandatory=$true)][string]$LabWorkspace,
  [Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9][a-z0-9-]{0,59}$')][string]$RunId
)

$ErrorActionPreference = 'Stop'
$labTools = Join-Path (Resolve-Path -LiteralPath $LabWorkspace).Path 'tools'
. (Join-Path $labTools 'benchmark-wsl.ps1')

$pilot = Join-Path $labTools 'benchmark-pilot.ps1'
if ($Action -eq 'Run' -and -not (Test-Path -LiteralPath $pilot -PathType Leaf)) {
  throw 'The local pilot launcher is missing.'
}

$source = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'benchmark-native-readiness.py') -Raw -Encoding UTF8
$bootstrap = @'
set -eu
probe=$(mktemp)
trap 'rm -f "$probe"' EXIT
cat > "$probe"
chmod 444 "$probe"
python3 -I -B "$probe" "$@"
'@
$checked = Invoke-BenchmarkWsl -NativeArguments @(
  '-d','JecodeBench-tb41-20261117','-u','root','--cd','/','--exec',
  '/bin/sh','-c',$bootstrap,'benchmark-readiness',$RunId,$Action.ToLowerInvariant()
) -InputText $source

if (-not $checked.Output) {
  throw 'Could not run the offline readiness check in the owned WSL laboratory.'
}
$receipt = $checked.Output | ConvertFrom-Json
Write-Output $checked.Output.Trim()
if ($checked.ExitCode -ne 0 -or $receipt.verdict -cne 'passed' -or $receipt.run_id -cne $RunId) {
  throw "Offline native readiness failed for $RunId. See the readiness receipt."
}

if ($Action -eq 'Run') {
  $arguments = @{
    Action = 'Run'
    TaskId = $receipt.task_id
    Profile = $receipt.profile
    RunId = $RunId
    Scope = $receipt.pilot_scope
    WithReactOsRuntime = [bool]$receipt.with_os_runtime
  }
  & $pilot @arguments
}
