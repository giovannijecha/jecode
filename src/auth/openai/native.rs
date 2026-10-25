//! OS crypto boundary. Unix requires the `openssl` executable; Windows uses
//! the installed Windows PowerShell and .NET cryptography APIs.

use crate::cancel::Cancellation;
use std::process::Command;
use std::time::Duration;

fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    cancel: &Cancellation,
) -> Result<Vec<u8>, String> {
    crate::auth::isolate_environment(command);
    let output = crate::process::run_observed(
        command,
        input,
        Some(Duration::from_secs(15)),
        64 * 1024,
        cancel,
        None,
        &mut |_| Ok(()),
    )?;
    if output.cancelled || cancel.requested() {
        return Err("Operation cancelled".into());
    }
    if output.timed_out
        || output.exit_code != Some(0)
        || output.stdout_truncated
        || output.input_error.is_some()
    {
        return Err("Native cryptography operation failed; check the required system tool".into());
    }
    Ok(output.stdout)
}

#[cfg(unix)]
pub fn random(count: usize, cancel: &Cancellation) -> Result<Vec<u8>, String> {
    let bytes = run(
        Command::new("openssl").args(["rand", &count.to_string()]),
        None,
        cancel,
    )?;
    if bytes.len() != count {
        return Err("Native random generator returned an invalid length".into());
    }
    Ok(bytes)
}

#[cfg(unix)]
pub fn sha256(data: &[u8], cancel: &Cancellation) -> Result<Vec<u8>, String> {
    let bytes = run(
        Command::new("openssl").args(["dgst", "-sha256", "-binary"]),
        Some(data.to_vec()),
        cancel,
    )?;
    if bytes.len() != 32 {
        return Err("Native SHA-256 returned an invalid length".into());
    }
    Ok(bytes)
}

#[cfg(unix)]
pub fn verify_rs256(
    data: &[u8],
    signature: &[u8],
    modulus: &[u8],
    exponent: &[u8],
    cancel: &Cancellation,
) -> Result<bool, String> {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut directory = std::env::temp_dir();
    directory.push(format!(
        "jecode-jwt-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory)
        .map_err(|_| "Could not create private signature workspace".to_string())?;
    fs::set_permissions(
        &directory,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .map_err(|_| "Could not protect signature workspace".to_string())?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let write = |name: &str, bytes: &[u8]| -> Result<std::path::PathBuf, String> {
        let path = directory.join(name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| "Could not create private signature input".to_string())?;
        file.write_all(bytes)
            .map_err(|_| "Could not write private signature input".to_string())?;
        Ok(path)
    };
    let key = write("key.pem", &pem(&rsa_public_der(modulus, exponent)))?;
    let sig = write("signature", signature)?;
    let message = write("message", data)?;
    let mut command = Command::new("openssl");
    command
        .args(["dgst", "-sha256", "-verify"])
        .arg(key)
        .arg("-signature")
        .arg(sig)
        .arg(message);
    crate::auth::isolate_environment(&mut command);
    let output = crate::process::run_observed(
        &mut command,
        None,
        Some(Duration::from_secs(15)),
        1024,
        cancel,
        None,
        &mut |_| Ok(()),
    )?;
    if output.cancelled || cancel.requested() {
        return Err("Operation cancelled".into());
    }
    if output.timed_out {
        return Err("Native signature verification timed out".into());
    }
    Ok(output.exit_code == Some(0) && output.stdout == b"Verified OK\n")
}

#[cfg(unix)]
fn der_length(length: usize) -> Vec<u8> {
    if length < 128 {
        return vec![length as u8];
    }
    let raw = length.to_be_bytes();
    let significant = raw
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(raw.len() - 1);
    let mut result = vec![0x80 | (raw.len() - significant) as u8];
    result.extend_from_slice(&raw[significant..]);
    result
}

#[cfg(unix)]
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut result = vec![tag];
    result.extend(der_length(content.len()));
    result.extend_from_slice(content);
    result
}

#[cfg(unix)]
fn integer(raw: &[u8]) -> Vec<u8> {
    let content = if raw.first().is_some_and(|b| b & 0x80 != 0) {
        let mut value = vec![0];
        value.extend_from_slice(raw);
        value
    } else {
        raw.to_vec()
    };
    der(0x02, &content)
}

#[cfg(unix)]
fn rsa_public_der(n: &[u8], e: &[u8]) -> Vec<u8> {
    let rsa = der(0x30, &[integer(n), integer(e)].concat());
    let algorithm = der(
        0x30,
        &[
            der(
                0x06,
                &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01],
            ),
            der(0x05, &[]),
        ]
        .concat(),
    );
    let mut bits = vec![0];
    bits.extend(rsa);
    der(0x30, &[algorithm, der(0x03, &bits)].concat())
}

#[cfg(unix)]
fn pem(der: &[u8]) -> Vec<u8> {
    let encoded = crate::attachments::base64::encode(der);
    let mut result = String::from("-----BEGIN PUBLIC KEY-----\n");
    for line in encoded.as_bytes().chunks(64) {
        result.push_str(std::str::from_utf8(line).unwrap());
        result.push('\n');
    }
    result.push_str("-----END PUBLIC KEY-----\n");
    result.into_bytes()
}

#[cfg(windows)]
fn powershell(
    input: crate::json::Value,
    cancel: &Cancellation,
) -> Result<crate::json::Value, String> {
    use std::sync::OnceLock;
    static HELPER: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    let executable = HELPER.get_or_init(|| {
        crate::process::compile_helper(
            "openai-native-v1",
            include_str!("native.cs"),
            &Cancellation::default(),
        )
    });
    let output = if let Some(executable) = executable {
        let op = input
            .get("op")
            .and_then(crate::json::Value::as_str)
            .ok_or("Native operation is missing")?;
        let mut lines = vec![op.to_string()];
        for field in match op {
            "random" => &["count"][..],
            "sha256" => &["data"][..],
            "verify" => &["data", "signature", "n", "e"][..],
            "protect_directory" | "protect_file" => &["path"][..],
            _ => return Err("Unknown native operation".into()),
        } {
            let value = input
                .get(field)
                .ok_or("Native operation field is missing")?;
            let raw = match value {
                crate::json::Value::String(text) => text.clone(),
                crate::json::Value::Number(number) => number.clone(),
                _ => return Err("Native operation field is invalid".into()),
            };
            lines.push(if *field == "path" {
                crate::attachments::base64::encode(raw.as_bytes())
            } else {
                raw
            });
        }
        let mut command = Command::new(executable);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(crate::process::CREATE_NO_WINDOW);
        }
        run(
            &mut command,
            Some(format!("{}\n", lines.join("\n")).into_bytes()),
            cancel,
        )?
    } else {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            include_str!("native.ps1"),
        ]);
        run(&mut command, Some(input.encode().into_bytes()), cancel)?
    };
    let text = String::from_utf8(output)
        .map_err(|_| "Native cryptography returned invalid UTF-8".to_string())?;
    crate::json::parse(text.trim())
        .map_err(|_| "Native cryptography returned invalid JSON".to_string())
}

#[cfg(windows)]
pub fn random(count: usize, cancel: &Cancellation) -> Result<Vec<u8>, String> {
    let value = powershell(
        crate::json::Value::object([
            ("op", crate::json::Value::string("random")),
            ("count", crate::json::Value::number(count)),
        ]),
        cancel,
    )?;
    let encoded = value
        .as_str()
        .ok_or("Native random generator returned invalid data")?;
    let bytes = crate::attachments::base64::decode(encoded)?;
    if bytes.len() != count {
        return Err("Native random generator returned an invalid length".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
pub fn sha256(data: &[u8], cancel: &Cancellation) -> Result<Vec<u8>, String> {
    let value = powershell(
        crate::json::Value::object([
            ("op", crate::json::Value::string("sha256")),
            (
                "data",
                crate::json::Value::string(crate::attachments::base64::encode(data)),
            ),
        ]),
        cancel,
    )?;
    let encoded = value
        .as_str()
        .ok_or("Native SHA-256 returned invalid data")?;
    let bytes = crate::attachments::base64::decode(encoded)?;
    if bytes.len() != 32 {
        return Err("Native SHA-256 returned an invalid length".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
pub fn verify_rs256(
    data: &[u8],
    signature: &[u8],
    modulus: &[u8],
    exponent: &[u8],
    cancel: &Cancellation,
) -> Result<bool, String> {
    let value = powershell(
        crate::json::Value::object([
            ("op", crate::json::Value::string("verify")),
            (
                "data",
                crate::json::Value::string(crate::attachments::base64::encode(data)),
            ),
            (
                "signature",
                crate::json::Value::string(crate::attachments::base64::encode(signature)),
            ),
            (
                "n",
                crate::json::Value::string(crate::attachments::base64::encode(modulus)),
            ),
            (
                "e",
                crate::json::Value::string(crate::attachments::base64::encode(exponent)),
            ),
        ]),
        cancel,
    )?;
    match value {
        crate::json::Value::Bool(valid) => Ok(valid),
        _ => Err("Native RSA verifier returned invalid data".into()),
    }
}

#[cfg(windows)]
pub fn protect(path: &std::path::Path, directory: bool) -> Result<(), String> {
    let value = powershell(
        crate::json::Value::object([
            (
                "op",
                crate::json::Value::string(if directory {
                    "protect_directory"
                } else {
                    "protect_file"
                }),
            ),
            ("path", crate::json::Value::string(path.to_string_lossy())),
        ]),
        &Cancellation::default(),
    )?;
    if value == crate::json::Value::Bool(true) {
        Ok(())
    } else {
        Err("Could not protect OpenAI credentials with owner-only ACL".into())
    }
}
