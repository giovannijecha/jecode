//! Native image boundary: reading an image from the system clipboard on an
//! explicit request, and converting images providers cannot take directly.
//!
//! Windows, and WSL through interop, use Windows PowerShell 5.1
//! (`powershell.exe`) with System.Windows.Forms and System.Drawing. macOS uses
//! `osascript` and `sips`. Native Linux uses an embedded helper that speaks
//! X11 or Wayland data-control and loads the desktop's imaging library.
//! Image bytes travel through stdin and stdout, never through arguments.

use crate::cancel::Cancellation;
use crate::process;
use std::process::Command;
use std::time::Duration;

const OUTPUT_LIMIT: usize = 96 * 1024 * 1024;

/// Reads the clipboard image as PNG bytes.
pub fn clipboard_image(cancellation: &Cancellation) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "linux")]
    if !super::paths::wsl() {
        let mut command = linux_command("clipboard")?;
        return run(&mut command, None, Duration::from_secs(20), cancellation);
    }
    let (mut command, encoding) = clipboard_command()?;
    let stdout = run(&mut command, None, Duration::from_secs(20), cancellation)?;
    match encoding {
        #[cfg(not(target_os = "macos"))]
        Encoding::Base64 => super::base64::decode(std::str::from_utf8(&stdout).unwrap_or("!")),
        #[cfg(target_os = "macos")]
        Encoding::AppleScript => apple_data(&stdout),
    }
}

/// Produces a provider-ready image at most 2048 pixels on its long side: PNG,
/// or quality-85 JPEG when the PNG exceeds 512 KiB and the JPEG is smaller.
/// Returns the file extension and the bytes.
pub fn convert(
    data: &[u8],
    cancellation: &Cancellation,
) -> Result<(&'static str, Vec<u8>), String> {
    convert_native(data, cancellation)
}

enum Encoding {
    #[cfg(not(target_os = "macos"))]
    Base64,
    #[cfg(target_os = "macos")]
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
        Some(0) => {
            if let Some(error) = output.input_error {
                Err(format!("{program} could not run: {error}"))
            } else {
                Ok(output.stdout)
            }
        }
        Some(3) => Err("The clipboard has no image".into()),
        #[cfg(target_os = "linux")]
        Some(4) => {
            Err("Desktop clipboard access is unavailable or denied by the compositor".into())
        }
        #[cfg(target_os = "linux")]
        Some(6) => Err("The desktop image library is unavailable".into()),
        _ => Err(format!("{program} failed")),
    }
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
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
$graphics.PixelOffsetMode = 'HighQuality'
$graphics.DrawImage($source, 0, 0, $width, $height)
$graphics.Dispose()
$output = New-Object System.IO.MemoryStream
$bitmap.Save($output, [System.Drawing.Imaging.ImageFormat]::Png)
$extension = 'png'
if ($output.Length -gt 524288) {
  # Transparent areas become white, as image viewers show them.
  $flat = New-Object System.Drawing.Bitmap $width, $height, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $canvas = [System.Drawing.Graphics]::FromImage($flat)
  $canvas.Clear([System.Drawing.Color]::White)
  $canvas.DrawImage($bitmap, 0, 0, $width, $height)
  $canvas.Dispose()
  $codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
  $parameters = New-Object System.Drawing.Imaging.EncoderParameters 1
  $parameters.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), 85L
  $jpeg = New-Object System.IO.MemoryStream
  $flat.Save($jpeg, $codec, $parameters)
  if ($jpeg.Length -lt $output.Length) {
    $output = $jpeg
    $extension = 'jpg'
  }
}
[Console]::Out.Write($extension + "`n" + [Convert]::ToBase64String($output.ToArray()))"#;

#[cfg(not(target_os = "macos"))]
fn powershell(script: &str) -> Command {
    let mut command = crate::native::powershell();
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

#[cfg(any(not(target_os = "macos"), test))]
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

#[cfg(windows)]
fn convert_native(
    data: &[u8],
    cancellation: &Cancellation,
) -> Result<(&'static str, Vec<u8>), String> {
    let stdout = run(
        &mut powershell(POWERSHELL_CONVERT),
        Some(data.to_vec()),
        Duration::from_secs(60),
        cancellation,
    )?;
    windows_converted(stdout)
}

#[cfg(target_os = "linux")]
fn convert_native(
    data: &[u8],
    cancellation: &Cancellation,
) -> Result<(&'static str, Vec<u8>), String> {
    if super::paths::wsl() {
        let stdout = run(
            &mut powershell(POWERSHELL_CONVERT),
            Some(data.to_vec()),
            Duration::from_secs(60),
            cancellation,
        )?;
        return windows_converted(stdout);
    }
    let mut command = linux_command("convert")?;
    linux_converted(run(
        &mut command,
        Some(data.to_vec()),
        Duration::from_secs(60),
        cancellation,
    )?)
}

#[cfg(any(target_os = "linux", test))]
fn linux_converted(stdout: Vec<u8>) -> Result<(&'static str, Vec<u8>), String> {
    let (kind, bytes) = stdout.split_at(stdout.len().min(4));
    let extension = match kind {
        b"png\n" => "png",
        b"jpg\n" => "jpg",
        _ => return Err("Image conversion failed".into()),
    };
    if bytes.is_empty() {
        return Err("Image conversion failed".into());
    }
    Ok((extension, bytes.to_vec()))
}

#[cfg(target_os = "linux")]
fn linux_command(action: &str) -> Result<Command, String> {
    let mut command = Command::new(crate::native::helper("clipboard_linux")?);
    command.arg(action);
    Ok(command)
}

#[cfg(windows)]
fn clipboard_command() -> Result<(Command, Encoding), String> {
    Ok((powershell(POWERSHELL_CLIPBOARD), Encoding::Base64))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn clipboard_command() -> Result<(Command, Encoding), String> {
    // Called only for WSL, whose clipboard is owned by Windows interop.
    Ok((powershell(POWERSHELL_CLIPBOARD), Encoding::Base64))
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
#[cfg(any(target_os = "macos", test))]
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
        let png = std::fs::read(&output).map_err(|error| error.to_string())?;
        if png.len() <= 512 * 1024 {
            return Ok(("png", png));
        }
        let jpeg = folder.join("output.jpg");
        let mut command = Command::new("/usr/bin/sips");
        command
            .args(["-s", "format", "jpeg", "-s", "formatOptions", "85"])
            .arg(&output)
            .arg("--out")
            .arg(&jpeg);
        run(&mut command, None, Duration::from_secs(60), cancellation)?;
        let smaller = std::fs::read(&jpeg).map_err(|error| error.to_string())?;
        Ok(if smaller.len() < png.len() {
            ("jpg", smaller)
        } else {
            ("png", png)
        })
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
        assert_eq!(
            linux_converted(b"png\n\x89PNG".to_vec()).unwrap(),
            ("png", b"\x89PNG".to_vec())
        );
        assert!(linux_converted(b"png\n".to_vec()).is_err());
        assert!(linux_converted(b"other\nimage".to_vec()).is_err());
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
    #[ignore = "runs the native image converter on the host"]
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
        // Detailed large images become a compact JPEG with the same geometry.
        let mut noise = bmp(3000, 1500);
        let mut state = 0x2545_f491_u32;
        for byte in &mut noise[54..] {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        let (extension, jpeg) = convert(&noise, &Cancellation::default()).unwrap();
        assert_eq!(extension, "jpg");
        let detected = crate::attachments::media::detect(&jpeg, "x", true);
        assert_eq!(
            (detected.media.as_str(), detected.width, detected.height),
            ("image/jpeg", Some(2048), Some(1024))
        );
    }

    #[test]
    #[ignore = "runs the native image converter on the host"]
    fn native_imports_skip_unused_or_too_large_views() {
        let (_, png) = convert(&bmp(3000, 10), &Cancellation::default()).unwrap();
        let root = crate::test_support::Directory::new();
        let pool = crate::attachments::Pool::new(root.path().to_path_buf());

        let mut large = png.clone();
        large.resize(16 * 1024 * 1024 + 1, 0);
        assert!(convert(&large, &Cancellation::default()).unwrap().1.len() < large.len());
        let image = pool.import_bytes("large.png", &large).unwrap();
        let stored = pool.load(&image.id).unwrap();
        assert_eq!(std::fs::read(stored.path).unwrap(), large);
        assert!(!root.path().join(format!("{}.view.png", image.id)).exists());
        assert!(!root.path().join(format!("{}.view.jpg", image.id)).exists());

        let mut page = png;
        page.resize(600_000, 0);
        assert!(convert(&page, &Cancellation::default()).unwrap().1.len() < page.len());
        let url = format!(
            "data:image/png;base64,{}",
            super::super::base64::encode(&page)
        );
        let mut message = crate::json::Value::object([(
            "annotations",
            crate::json::Value::Array(vec![crate::json::Value::string(&url)]),
        )]);
        crate::backend::openrouter::annotations::store(&mut message, Some(&pool)).unwrap();
        let reference = message
            .get("annotations")
            .and_then(crate::json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("jecode_attachment"))
            .and_then(crate::json::Value::as_str)
            .unwrap();
        let id = reference.strip_prefix("attachment:").unwrap();
        assert_eq!(std::fs::read(pool.load(id).unwrap().path).unwrap(), page);
        assert!(!root.path().join(format!("{id}.view.png")).exists());
        assert!(!root.path().join(format!("{id}.view.jpg")).exists());
        crate::backend::openrouter::annotations::restore(&mut message, Some(&pool)).unwrap();
        assert_eq!(
            message
                .get("annotations")
                .and_then(crate::json::Value::as_array)
                .and_then(|items| items.first())
                .and_then(crate::json::Value::as_str),
            Some(url.as_str())
        );
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
