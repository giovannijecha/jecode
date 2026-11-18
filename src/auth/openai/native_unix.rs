use crate::cancel::Cancellation;
use crate::process::Output;
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
    checked_output(output, cancel)
}

fn checked_output(output: Output, cancel: &Cancellation) -> Result<Vec<u8>, String> {
    if output.cancelled || cancel.requested() {
        return Err("Operation cancelled".into());
    }
    if output.timed_out {
        return Err("Native cryptography operation timed out".into());
    }
    // linux_crypto.c reserves this status for a libcrypto load failure.
    #[cfg(target_os = "linux")]
    if output.exit_code == Some(2) {
        return Err(
            "OS libcrypto unavailable. Install or repair the system OpenSSL runtime \
             (libcrypto.so.3 or libcrypto.so.1.1)."
                .into(),
        );
    }
    if output.exit_code != Some(0) || output.stdout_truncated {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_output(exit_code: i32) -> Output {
        let stderr = b"OS libcrypto unavailable\nsynthetic-secret\n".to_vec();
        Output {
            stdout: Vec::new(),
            stderr_bytes: stderr.len(),
            stderr,
            exit_code: Some(exit_code),
            input_error: None,
            timed_out: false,
            stdout_bytes: 0,
            stdout_truncated: false,
            cancelled: false,
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn libcrypto_load_failure_uses_a_fixed_local_diagnostic() {
        let error = checked_output(failed_output(2), &Cancellation::default()).unwrap_err();
        assert!(error.starts_with("OS libcrypto unavailable."));
        assert!(error.contains("libcrypto.so.3"));
        assert!(!error.contains("synthetic-secret"));
    }

    #[test]
    fn other_failures_return_a_generic_diagnostic() {
        let error = checked_output(failed_output(1), &Cancellation::default()).unwrap_err();
        assert_eq!(error, "Native cryptography operation failed");
    }

    #[test]
    fn timeout_and_cancellation_take_precedence_over_failure_status() {
        let mut timed_out = failed_output(2);
        timed_out.timed_out = true;
        assert_eq!(
            checked_output(timed_out, &Cancellation::default()).unwrap_err(),
            "Native cryptography operation timed out"
        );

        let mut cancelled = failed_output(2);
        cancelled.timed_out = true;
        cancelled.cancelled = true;
        assert_eq!(
            checked_output(cancelled, &Cancellation::default()).unwrap_err(),
            "Operation cancelled"
        );

        let cancellation = Cancellation::default();
        cancellation.cancel();
        assert_eq!(
            sha256(b"abc", &cancellation).unwrap_err(),
            "Operation cancelled"
        );
    }
}
