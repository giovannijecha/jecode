use crate::cancel::Cancellation;
use crate::process;
use std::process::Command;
use std::time::Duration;

#[test]
fn native_slow_file_drop_and_split_overflow_remain_atomic() {
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n",
        include_str!("../../terminal.cs")
    ) + r#"
function Send([JecodeConsole+Burst]$burst, [IO.StringWriter]$writer, [string]$text, [long]$time) {
    $records = New-Object JecodeConsole+Record[] $text.Length
    for ($index = 0; $index -lt $text.Length; $index++) {
        $record = New-Object JecodeConsole+Record
        $record.Type = 1; $record.Down = 1; $record.Repeat = 1
        $record.Character = [uint16][char]$text[$index]
        $records[$index] = $record
    }
    $burst.Emit($records, [uint32]$records.Length, $writer, $time)
}
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
Send $burst $writer '"C:\fixture\' 0
$burst.Idle($writer, 5)
Write-Output ($writer.ToString().Length -eq 0)
Send $burst $writer 'beta.txt"' 10
$burst.Idle($writer, 60)
Write-Output ($writer.ToString().Length -eq 0)
$burst.Idle($writer, 61)
$lines = @($writer.ToString().Trim() -split "`n")
Write-Output ($lines.Length -eq 1 -and $lines[0].StartsWith('P|'))
$path = -join ($lines[0].Substring(2).Split(',') | ForEach-Object { [char][int]$_ })
Write-Output ($path -eq '"C:\fixture\beta.txt"')

$record = New-Object JecodeConsole+Record
$record.Type = 1; $record.Down = 1; $record.Repeat = 65535
$record.Key = 88; $record.Character = 120
$first = New-Object JecodeConsole+Record[] 8
for ($index = 0; $index -lt 8; $index++) { $first[$index] = $record }
$second = New-Object JecodeConsole+Record[] 9
[array]::Copy($first, $second, 8)
$record.Repeat = 18; $second[8] = $record
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
$burst.Emit($first, 8, $writer, 0)
$burst.Idle($writer, 5)
Write-Output ($writer.ToString().Length -eq 0)
$burst.Emit($second, 9, $writer, 10)
$burst.Idle($writer, 61)
Write-Output ($writer.ToString().Trim() -eq 'I|overflow')
"#;
    let output = process::run_observed(
        Command::new("powershell.exe").args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]),
        None,
        Duration::from_secs(20),
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["True", "True", "True", "True", "True", "True"]
    );
}
