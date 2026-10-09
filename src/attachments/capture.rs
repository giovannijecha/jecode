//! Native image boundary: reading an image from the system clipboard on an
//! explicit request, and converting images providers cannot take directly.
//!
//! Windows, and WSL through interop, use Windows PowerShell 5.1
//! (`powershell.exe`) with System.Windows.Forms and System.Drawing. macOS uses
//! `osascript` and `sips`. Other Unix systems read the clipboard with
//! `wl-paste` (Wayland) or `xclip` (X11) and have no converter. Image bytes
//! travel through stdin and stdout, never through arguments or files.

use crate::cancel::Cancellation;
use crate::process;
use std::process::Command;
use std::time::Duration;

const OUTPUT_LIMIT: usize = 96 * 1024 * 1024;

/// Reads the clipboard image as PNG bytes.
pub fn clipboard_image(cancellation: &Cancellation) -> Result<Vec<u8>, String> {
    let (mut command, encoding) = clipboard_command()?;
    let stdout = run(&mut command, None, Duration::from_secs(20), cancellation)?;
    match encoding {
        Encoding::Base64 => super::base64::decode(std::str::from_utf8(&stdout).unwrap_or("!")),
        Encoding::Raw => Ok(stdout),
        Encoding::AppleScript => apple_data(&stdout),
    }
}

/// Produces a provider-ready PNG, or JPEG for large photos, at most 2048
/// pixels on its long side. Returns the file extension and the bytes.
pub fn convert(
    data: &[u8],
    cancellation: &Cancellation,
) -> Result<(&'static str, Vec<u8>), String> {
    convert_native(data, cancellation)
}

enum Encoding {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Base64,
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    Raw,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    AppleScript,
}

fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = process::run_observed(
        command,
        input,
        timeout,
        OUTPUT_LIMIT,
        cancellation,
        None,
        &mut |_| Ok(()),
    )
    .map_err(|error| format!("{program} could not run: {error}"))?;
    if output.cancelled {
        return Err("Operation cancelled".into());
    }
    if output.timed_out {
        return Err(format!("{program} timed out"));
    }
    if output.stdout_truncated {
        return Err("The image is too large".into());
    }
    match output.exit_code {
        Some(0) => Ok(output.stdout),
        Some(3) => Err("The clipboard has no image".into()),
        _ => Err(format!("{program} failed")),
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
const POWERSHELL_CLIPBOARD: &str = r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$data = [System.Windows.Forms.Clipboard]::GetDataObject()
$bytes = $null
if ($null -ne $data -and $data.GetDataPresent('PNG')) {
  $stream = $data.GetData('PNG')
  if ($stream -is [System.IO.Stream]) {
    $memory = New-Object System.IO.MemoryStream
    $stream.CopyTo($memory)
    $bytes = $memory.ToArray()
  }
}
if ($null -eq $bytes -and [System.Windows.Forms.Clipboard]::ContainsImage()) {
  $image = [System.Windows.Forms.Clipboard]::GetImage()
  $memory = New-Object System.IO.MemoryStream
  $image.Save($memory, [System.Drawing.Imaging.ImageFormat]::Png)
  $bytes = $memory.ToArray()
}
if ($null -eq $bytes) { exit 3 }
[Console]::Out.Write([Convert]::ToBase64String($bytes))"#;

#[cfg_attr(target_os = "macos", allow(dead_code))]
const POWERSHELL_CONVERT: &str = r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$buffer = New-Object System.IO.MemoryStream
[Console]::OpenStandardInput().CopyTo($buffer)
$buffer.Position = 0
$source = [System.Drawing.Image]::FromStream($buffer)
$scale = [Math]::Min(1.0, 2048.0 / [Math]::Max($source.Width, $source.Height))
$width = [Math]::Max(1, [int]($source.Width * $scale))
$height = [Math]::Max(1, [int]($source.Height * $scale))
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.InterpolationMode = 'HighQualityBicubic'
$graphics.DrawImage($source, 0, 0, $width, $height)
$graphics.Dispose()
$output = New-Object System.IO.MemoryStream
$bitmap.Save($output, [System.Drawing.Imaging.ImageFormat]::Png)
$extension = 'png'
if ($output.Length -gt 3750000) {
  $output = New-Object System.IO.MemoryStream
  $bitmap.Save($output, [System.Drawing.Imaging.ImageFormat]::Jpeg)
  $extension = 'jpg'
}
[Console]::Out.Write($extension + "`n" + [Convert]::ToBase64String($output.ToArray()))"#;

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn powershell(script: &str) -> Command {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Sta",
        "-Command",
        script,
    ]);
    command
}

#[cfg(not(target_os = "macos"))]
fn powershell_available() -> Result<(), String> {
    #[cfg(not(windows))]
    if !super::paths::wsl() {
        return Err("No image converter is available on this platform".into());
    }
    Ok(())
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn windows_converted(stdout: Vec<u8>) -> Result<(&'static str, Vec<u8>), String> {
    let text = String::from_utf8(stdout).map_err(|_| "Image conversion failed")?;
    let (extension, data) = text.split_once('\n').ok_or("Image conversion failed")?;
    let extension = match extension.trim() {
        "png" => "png",
        "jpg" => "jpg",
        _ => return Err("Image conversion failed".into()),
    };
    Ok((extension, super::base64::decode(data)?))
}

#[cfg(not(target_os = "macos"))]
fn convert_native(
    data: &[u8],
    cancellation: &Cancellation,
) -> Result<(&'static str, Vec<u8>), String> {
    powershell_available()?;
    let stdout = run(
        &mut powershell(POWERSHELL_CONVERT),
        Some(data.to_vec()),
        Duration::from_secs(60),
        cancellation,
    )?;
    windows_converted(stdout)
}

#[cfg(windows)]
fn clipboard_command() -> Result<(Command, Encoding), String> {
    Ok((powershell(POWERSHELL_CLIPBOARD), Encoding::Base64))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn clipboard_command() -> Result<(Command, Encoding), String> {
    if super::paths::wsl() {
        // Windows interop; fails visibly when interop or the Windows PATH is disabled.
        return Ok((powershell(POWERSHELL_CLIPBOARD), Encoding::Base64));
    }
    let mut command = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let mut command = Command::new("wl-paste");
        command.args(["--no-newline", "--type", "image/png"]);
        command
    } else if std::env::var_os("DISPLAY").is_some() {
        let mut command = Command::new("xclip");
        command.args(["-selection", "clipboard", "-target", "image/png", "-out"]);
        command
    } else {
        return Err("No graphical clipboard is available".into());
    };
    command.env("LC_ALL", "C");
    Ok((command, Encoding::Raw))
}

#[cfg(target_os = "macos")]
fn clipboard_command() -> Result<(Command, Encoding), String> {
    let mut command = Command::new("/usr/bin/osascript");
    command.args([
        "-e",
        "try",
        "-e",
        "the clipboard as «class PNGf»",
        "-e",
        "on error",
        "-e",
        "return \"\"",
        "-e",
        "end try",
    ]);
    Ok((command, Encoding::AppleScript))
}

/// Parses AppleScript's `«data PNGf…»` hex rendering.
fn apple_data(stdout: &[u8]) -> Result<Vec<u8>, String> {
    let text = String::from_utf8_lossy(stdout);
    let hex = text
        .trim()
        .strip_prefix("«data PNGf")
        .and_then(|rest| rest.strip_suffix('»'))
        .ok_or("The clipboard has no image")?;
    (0..hex.len())
        .step_by(2)
        .map(|at| {
            hex.get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| "Clipboard image data is damaged".to_string())
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn convert_native(
    data: &[u8],
    cancellation: &Cancellation,
) -> Result<(&'static str, Vec<u8>), String> {
    let folder = std::env::temp_dir().join(format!("jecode-{}", super::new_id()));
    std::fs::create_dir(&folder).map_err(|error| error.to_string())?;
    let result = (|| {
        let input = folder.join("input");
        let output = folder.join("output.png");
        std::fs::write(&input, data).map_err(|error| error.to_string())?;
        let mut command = Command::new("/usr/bin/sips");
        command
            .args(["-s", "format", "png", "-Z", "2048"])
            .arg(&input)
            .arg("--out")
            .arg(&output);
        run(&mut command, None, Duration::from_secs(60), cancellation)?;
        Ok((
            "png",
            std::fs::read(&output).map_err(|error| error.to_string())?,
        ))
    })();
    let _ = std::fs::remove_dir_all(&folder);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_native_output_formats() {
        assert_eq!(
            apple_data("«data PNGf89504E47»\n".as_bytes()).unwrap(),
            [0x89, b'P', b'N', b'G']
        );
        assert!(apple_data(b"missing value").is_err());
        assert_eq!(
            windows_converted(b"jpg\n/9j/".to_vec()).unwrap(),
            ("jpg", vec![0xff, 0xd8, 0xff])
        );
        assert!(windows_converted(b"exe\nAAAA".to_vec()).is_err());
    }

    /// An uncompressed 24-bit BMP, a format providers do not take directly.
    fn bmp(width: u32, height: u32) -> Vec<u8> {
        let row = (width * 3).div_ceil(4) * 4;
        let size = 54 + row * height;
        let mut bytes = b"BM".to_vec();
        for value in [size, 0, 54, 40, width, height] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(24u16.to_le_bytes());
        bytes.extend([0; 24]);
        bytes.resize(size as usize, 0x7f);
        bytes
    }

    #[test]
    #[ignore = "runs the native image converter (PowerShell on Windows and WSL, sips on macOS)"]
    fn native_conversion_bounds_unsupported_images() {
        let (extension, png) = convert(&bmp(3000, 10), &Cancellation::default()).unwrap();
        assert_eq!(extension, "png");
        let detected = crate::attachments::media::detect(&png, "x", true);
        assert_eq!(
            (detected.media.as_str(), detected.width, detected.height),
            ("image/png", Some(2048), Some(7))
        );
        let root = crate::test_support::Directory::new();
        let pool = crate::attachments::Pool::new(root.path().to_path_buf());
        let attachment = pool.import_bytes("scan.bmp", &bmp(5, 4)).unwrap();
        assert_eq!(attachment.media, "image/bmp");
        let (media, view) = pool.view(&attachment).unwrap().unwrap();
        assert_eq!(media, "image/png");
        assert_eq!(
            crate::attachments::media::detect(&view, "x", true).width,
            Some(5)
        );
        assert!(convert(b"not an image", &Cancellation::default()).is_err());
    }

    #[test]
    #[ignore = "reads the real system clipboard; prints only the outcome"]
    fn native_clipboard_read_reports_its_outcome() {
        match clipboard_image(&Cancellation::default()) {
            Ok(bytes) => {
                let detected = crate::attachments::media::detect(&bytes, "x", true);
                assert_eq!(detected.media, "image/png");
                eprintln!("clipboard image: {} bytes", bytes.len());
            }
            Err(error) => {
                assert_eq!(error, "The clipboard has no image");
                eprintln!("clipboard: {error}");
            }
        }
    }
}
