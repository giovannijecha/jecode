#[cfg(windows)]
pub(super) fn encoded(script: &str) -> String {
    // Parse the model's script inside try/catch. A parser error in the enclosing
    // EncodedCommand would bypass the catch and leak PowerShell's CLIXML stderr.
    // Base64 keeps the script literal without passing through shell quoting.
    let literal = base64(script.as_bytes());
    let source = format!(
        concat!(
            "$ProgressPreference='SilentlyContinue';",
            "$ErrorActionPreference='Stop';",
            "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);",
            "$OutputEncoding=[Console]::OutputEncoding;",
            "$global:LASTEXITCODE=0;try {{",
            "$jecodeCwd=[Environment]::CurrentDirectory;",
            "Set-Location -LiteralPath $jecodeCwd;",
            "& ([scriptblock]::Create(",
            "[Text.Encoding]::UTF8.GetString(",
            "[Convert]::FromBase64String('{literal}'))));",
            "$jecodeSucceeded=$?;",
            "if($LASTEXITCODE -ne 0){{exit $LASTEXITCODE}};",
            "if(!$jecodeSucceeded){{exit 1}}",
            "}}catch{{[Console]::Error.WriteLine($_.ToString());exit 1}}"
        ),
        literal = literal
    );
    let bytes: Vec<u8> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

#[cfg(windows)]
fn base64(bytes: &[u8]) -> String {
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
