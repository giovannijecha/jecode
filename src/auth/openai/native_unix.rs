use crate::cancel::Cancellation;
use std::process::Command;
use std::time::Duration;

fn operation(op: u8, fields: &[&[u8]], cancel: &Cancellation) -> Result<Vec<u8>, String> {
    if cancel.requested() {
        return Err("Operation cancelled".into());
    }
    let mut input = vec![op];
    for field in fields {
        let length = u32::try_from(field.len()).map_err(|_| "Native crypto input too long")?;
        input.extend_from_slice(&length.to_be_bytes());
        input.extend_from_slice(field);
    }
    let mut command = Command::new(crate::native::helper("openai_crypto")?);
    crate::auth::isolate_environment(&mut command);
    let output = crate::process::run_leaf(
        &mut command,
        Some(input),
        Some(Duration::from_secs(15)),
        4096,
        cancel,
        None,
        &mut |_| Ok(()),
    )?;
    if output.cancelled || cancel.requested() {
        return Err("Operation cancelled".into());
    }
    if output.timed_out || output.exit_code != Some(0) || output.stdout_truncated {
        return Err("Native cryptography operation failed".into());
    }
    Ok(output.stdout)
}

pub fn random(count: usize, cancel: &Cancellation) -> Result<Vec<u8>, String> {
    let length = u32::try_from(count).map_err(|_| "Random count too large")?;
    let data = operation(b'r', &[&length.to_be_bytes()], cancel)?;
    if data.len() != count {
        return Err("Native random generator returned an invalid length".into());
    }
    Ok(data)
}

pub fn sha256(data: &[u8], cancel: &Cancellation) -> Result<Vec<u8>, String> {
    let digest = operation(b'h', &[data], cancel)?;
    if digest.len() != 32 {
        return Err("Native SHA-256 returned an invalid length".into());
    }
    Ok(digest)
}

pub fn verify_rs256(
    data: &[u8],
    signature: &[u8],
    modulus: &[u8],
    exponent: &[u8],
    cancel: &Cancellation,
) -> Result<bool, String> {
    let answer = operation(b'v', &[data, signature, modulus, exponent], cancel)?;
    match answer.as_slice() {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err("Native RSA verifier returned invalid data".into()),
    }
}
