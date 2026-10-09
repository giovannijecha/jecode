use super::*;
use crate::cancel::Cancellation;
use crate::process;

#[test]
fn native_records_preserve_unicode_and_distinguish_paste_from_shortcuts() {
    let script = format!(
        r#"
Add-Type -TypeDefinition @'
{}
'@
$records = New-Object JecodeConsole+Record[] 4
$record = New-Object JecodeConsole+Record
$record.Type = 1; $record.Down = 1; $record.Key = 65; $record.Character = 97; $records[0] = $record
$record.Down = 0; $records[1] = $record
$record.Key = 18; $record.Character = 55357; $records[2] = $record
$record.Character = 56898; $records[3] = $record
$keys = [JecodeConsole]::Keys($records, 4)
Write-Output (($keys | ForEach-Object {{ $_.Character }}) -join ',')
$record.Type = 1; $record.Down = 1; $record.Key = 65; $record.Character = 97; $record.Controls = 0; $records[0] = $record
$record.Key = 13; $record.Character = 13; $records[1] = $record
Write-Output ([JecodeConsole]::BufferedText([JecodeConsole]::Keys($records, 2), $false))
$record.Key = 9; $record.Character = 9; $records[0] = $record
Write-Output ([JecodeConsole]::BufferedText([JecodeConsole]::Keys($records, 2), $false))
$record.Key = 50; $record.Character = 64; $record.Controls = 9; $records[0] = $record
Write-Output ([JecodeConsole]::BufferedText([JecodeConsole]::Keys($records, 2), $false))
$record.Key = 67; $record.Character = 3; $record.Controls = 8; $records[0] = $record
Write-Output ([JecodeConsole]::BufferedText([JecodeConsole]::Keys($records, 2), $false))
$record.Type = 2; $record.MouseFlags = 4; $record.MouseButtons = 7864320
Write-Output ([JecodeConsole]::Wheel($record))
$record.MouseButtons = 4287102976
Write-Output ([JecodeConsole]::Wheel($record))
$record.MouseFlags = 0
Write-Output ([JecodeConsole]::Wheel($record))
$record.Type = 1; $record.Down = 1; $record.Key = 65; $record.Character = 97; $record.Controls = 0; $records[0] = $record
$record.Type = 2; $record.MouseFlags = 4; $record.MouseButtons = 7864320; $records[1] = $record
$record.Type = 1; $record.Down = 1; $record.Key = 13; $record.Character = 13; $record.Controls = 0; $records[2] = $record
$text = [Text.StringBuilder]::new(); $writer = [IO.StringWriter]::new($text)
$paste = $false; $last = [long]-1000; $wheel = 0
[JecodeConsole]::Emit($records, 3, $writer, [ref]$paste, [ref]$last, [ref]$wheel, 0)
Write-Output ($text.ToString().Trim())
"#,
        include_str!("../terminal.cs")
    );
    let output = process::run_observed(
        Command::new("powershell.exe").args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]),
        None,
        Duration::from_secs(10),
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
        [
            "97,55357,56898",
            "True",
            "False",
            "True",
            "False",
            "120",
            "-120",
            "0",
            "K|65|0|97",
            "W|-3",
            "K|13|0|13"
        ]
    );
}

#[test]
fn native_burst_joins_console_reads_and_keeps_keys_in_order() {
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n",
        include_str!("../terminal.cs")
    ) + r#"
function Send([JecodeConsole+Burst]$burst, [IO.StringWriter]$writer, [string]$text, [long]$time) {
    $records = New-Object JecodeConsole+Record[] $text.Length
    for ($index = 0; $index -lt $text.Length; $index++) {
        $record = New-Object JecodeConsole+Record
        $record.Type = 1; $record.Down = 1; $record.Repeat = 1
        $record.Key = if ($text[$index] -eq [char]13) { 13 } else { 0 }
        $record.Character = [uint16][char]$text[$index]
        $records[$index] = $record
    }
    $burst.Emit($records, [uint32]$records.Length, $writer, $time)
}
function PasteText([IO.StringWriter]$writer) {
    $line = $writer.ToString().Trim()
    if (-not $line.StartsWith('P|')) { return 'not a paste' }
    return -join ($line.Substring(2).Split(',') | ForEach-Object { [char][int]$_ })
}
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
Send $burst $writer '"C:\fixture\' 0
Send $burst $writer 'beta.txt"' 10
$burst.Idle($writer, 61)
Write-Output ((PasteText $writer) -eq '"C:\fixture\beta.txt"')
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
Send $burst $writer 'a' 0
Send $burst $writer "`r" 10
$burst.Idle($writer, 61)
Write-Output ($writer.ToString().Trim() -replace "`r", '' -replace "`n", ';')
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
Send $burst $writer 'first' 0
Send $burst $writer "`r`nsecond" 10
$burst.Idle($writer, 61)
Write-Output ((PasteText $writer) -eq "first`r`nsecond")
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
Send $burst $writer 'ab' 0
$shortcut = New-Object JecodeConsole+Record
$shortcut.Type = 1; $shortcut.Down = 1; $shortcut.Key = 81
$shortcut.Character = 17; $shortcut.Controls = 8
$wheel = New-Object JecodeConsole+Record
$wheel.Type = 2; $wheel.MouseFlags = 4; $wheel.MouseButtons = 7864320
$burst.Emit(@($shortcut, $wheel), 2, $writer, 10)
Write-Output ($writer.ToString().Trim() -replace "`r", '' -replace "`n", ';')
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
$large = New-Object JecodeConsole+Record[] 17
for ($index = 0; $index -lt $large.Length; $index++) {
    $record = New-Object JecodeConsole+Record
    $record.Type = 1; $record.Down = 1; $record.Repeat = 65535
    $record.Key = 88; $record.Character = 120
    $large[$index] = $record
}
$burst.Emit($large, [uint32]$large.Length, $writer, 0)
$burst.Idle($writer, 60)
Send $burst $writer 'a' 70
$burst.Idle($writer, 130)
Write-Output ($writer.ToString().Trim() -replace "`r", '' -replace "`n", ';')
$record = $large[16]; $record.Repeat = 17; $large[16] = $record
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
$burst.Emit($large, [uint32]$large.Length, $writer, 0)
$burst.Idle($writer, 60)
Write-Output $writer.ToString().Trim()
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
        Duration::from_secs(10),
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
        [
            "True",
            "K|0|0|97;K|13|0|13",
            "True",
            "P|97,98;K|81|4|17;W|-3",
            "I|overflow;K|0|0|97",
            "I|overflow",
        ]
    );
}

#[test]
fn native_alt_numpad_character_keeps_pasted_path_atomic() {
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@\n",
        include_str!("../terminal.cs")
    ) + r#"
function Record([int]$down, [int]$key, [int]$character, [int]$controls) {
    $record = New-Object JecodeConsole+Record
    $record.Type = 1; $record.Down = $down; $record.Repeat = 1
    $record.Key = $key; $record.Character = $character; $record.Controls = $controls
    return $record
}
$records = New-Object JecodeConsole+Record[] 8
$records[0] = Record 1 65 97 0
$records[1] = Record 1 18 0 2
$records[2] = Record 1 105 0 2
$records[3] = Record 0 105 0 2
$records[4] = Record 1 102 0 2
$records[5] = Record 0 102 0 2
$records[6] = Record 0 18 96 0
$records[7] = Record 1 66 98 0
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
$burst.Emit($records, [uint32]$records.Length, $writer, 0)
$burst.Idle($writer, 60)
Write-Output $writer.ToString().Trim()
$other = @( (Record 1 70 0 2), (Record 1 105 0 0) )
$burst = New-Object JecodeConsole+Burst
$writer = [IO.StringWriter]::new()
$burst.Emit($other, [uint32]$other.Length, $writer, 0)
Write-Output ($writer.ToString().Trim() -replace "`r", '' -replace "`n", ';')
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
        Duration::from_secs(10),
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
        ["P|97,96,98", "K|70|1|0;K|105|0|0"]
    );
}

#[test]
fn decodes_native_modifiers_and_unicode_paste() {
    assert!(matches!(parse("I|overflow"), Input::PasteOverflow));
    assert!(matches!(parse("W|-3"), Input::Scroll(-3)));
    assert!(matches!(parse("W|3"), Input::Scroll(3)));
    assert!(matches!(parse("W|invalid"), Input::Error(_)));
    assert!(matches!(parse("R|503|7|437"), Input::Modes(503, 7, 437)));
    assert!(matches!(parse("K|13|2|13"), Input::Key(key) if key.shift() && !key.ctrl()));
    assert!(matches!(parse("K|67|4|3"), Input::Key(key) if key.ctrl() && !key.shift()));
    assert!(
        matches!(parse("P|55357,56898,9,13,10"), Input::Paste(units) if units == [55357, 56898, 9, 13, 10])
    );
    assert!(matches!(
        parse("S|32|12|5|9"),
        Input::Size(Geometry {
            width: 32,
            height: 12,
            row: 5,
            column: 9
        })
    ));
    assert!(matches!(parse("S|0|12|5|9"), Input::Error(_)));
    assert!(matches!(parse("S|32|12"), Input::Error(_)));
    assert!(matches!(parse("S|32|12|bad|9"), Input::Error(_)));
}
