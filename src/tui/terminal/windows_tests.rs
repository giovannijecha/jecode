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
fn decodes_native_modifiers_and_unicode_paste() {
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
