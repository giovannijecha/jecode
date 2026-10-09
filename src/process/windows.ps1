$ErrorActionPreference = 'Stop'
try {
    Add-Type -TypeDefinition @'
__JECODE_NATIVE_SOURCE__
'@
    $code = [JecodeProcess]::Run()
    exit $code
} catch {
    $message = [Text.Encoding]::UTF8.GetBytes("Windows process supervisor failed: $($_.Exception.Message)")
    $output = [Console]::OpenStandardOutput()
    $output.WriteByte(0)
    $length = [BitConverter]::GetBytes([int]$message.Length)
    $output.Write($length, 0, $length.Length)
    $output.Write($message, 0, $message.Length)
    $output.Flush()
    exit 1
}
