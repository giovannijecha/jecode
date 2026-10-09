$ErrorActionPreference = 'Stop'
$source = @'
__JECODE_NATIVE_SOURCE__
'@
$pipe = $null
$writer = $null
try {
    Add-Type -TypeDefinition $source
    $pipe = [IO.Pipes.NamedPipeServerStream]::new('__JECODE_CHANNEL__', [IO.Pipes.PipeDirection]::Out, 1, [IO.Pipes.PipeTransmissionMode]::Byte, [IO.Pipes.PipeOptions]::Asynchronous)
    $pipe.WaitForConnection()
    $writer = [IO.StreamWriter]::new($pipe, [Text.UTF8Encoding]::new($false), 1024, $true)
    $writer.NewLine = "`n"
    $writer.AutoFlush = $true
    [JecodeConsole]::Run([Console]::OpenStandardInput(), $writer)
} catch {
    if ($null -ne $writer) { $writer.WriteLine('E|Terminal input failed: ' + $_.Exception.Message) }
} finally {
    if ($null -ne $writer) { $writer.Dispose() }
    if ($null -ne $pipe) { $pipe.Dispose() }
}
