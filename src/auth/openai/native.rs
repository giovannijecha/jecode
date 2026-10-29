//! OS cryptography boundary.

#[cfg(windows)]
use crate::cancel::Cancellation;
#[cfg(windows)]
use std::process::Command;
#[cfg(windows)]
use std::time::Duration;

#[cfg(all(test, windows))]
#[path = "native_tests.rs"]
mod tests;

#[cfg(windows)]
fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    cancel: &Cancellation,
) -> Result<Vec<u8>, String> {
    crate::auth::isolate_environment(command);
    let output = crate::process::run_leaf(
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
        return Err("Native cryptography operation failed".into());
    }
    Ok(output.stdout)
}

#[cfg(unix)]
#[path = "native_unix.rs"]
mod unix;
#[cfg(unix)]
pub use unix::{random, sha256, verify_rs256};

#[cfg(windows)]
fn powershell(
    input: crate::json::Value,
    cancel: &Cancellation,
) -> Result<crate::json::Value, String> {
    let executable = crate::native::helper("openai_crypto")?;
    let output = {
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
        let mut command = Command::new(&executable);
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
