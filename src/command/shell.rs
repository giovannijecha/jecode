#[cfg(windows)]
pub(super) const NAME: &str = "Windows PowerShell / no profile";
#[cfg(not(windows))]
pub(super) const NAME: &str = "/bin/sh / non-interactive";

#[cfg(windows)]
pub(super) fn encoded(script: &str) -> String {
    // The command is transported intact as UTF-16LE, never interpolated into
    // another command's quoting rules. Stop non-terminating PowerShell errors.
    let source = format!(
        "$ProgressPreference='SilentlyContinue';$ErrorActionPreference='Stop';[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$OutputEncoding=[Console]::OutputEncoding;$global:LASTEXITCODE=0;try {{ & {{\n{script}\n}}; $jecodeSucceeded=$?;if($LASTEXITCODE -ne 0){{exit $LASTEXITCODE}};if(!$jecodeSucceeded){{exit 1}} }} catch {{[Console]::Error.WriteLine($_.ToString());exit 1}}"
    );
    let bytes: Vec<u8> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            result.push(if i > chunk.len() {
                '='
            } else {
                ALPHABET[((n >> shift) & 63) as usize] as char
            });
        }
    }
    result
}
