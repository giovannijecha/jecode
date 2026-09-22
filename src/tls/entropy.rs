//! Native OS entropy only. No process, seed file, clock or weak fallback.
use super::{Error, KeyShare, crypto::secret::erase};

struct Material([u8; 64]);
impl Drop for Material {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}

pub(super) fn generate() -> Result<(KeyShare, [u8; 32]), Error> {
    let mut material = Material([0; 64]);
    fill(&mut material.0)?;
    let key = KeyShare::from_bytes(material.0[..32].try_into().unwrap());
    let random = material.0[32..].try_into().unwrap();
    Ok((key, random))
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn fill(output: &mut [u8; 64]) -> Result<(), Error> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            buffer: *mut u8,
            length: u32,
            flags: u32,
        ) -> i32;
    }
    // SAFETY: A writable exclusive 64-byte buffer remains live for the call.
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG (2) requires a null algorithm handle.
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), output.as_mut_ptr(), 64, 2) };
    if status >= 0 {
        Ok(())
    } else {
        Err(Error::Entropy)
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn fill(output: &mut [u8; 64]) -> Result<(), Error> {
    unsafe extern "C" {
        fn getrandom(buffer: *mut std::ffi::c_void, length: usize, flags: u32) -> isize;
    }
    let mut written = 0;
    let mut interruptions = 0;
    while written < output.len() {
        // SAFETY: The pointer names the remaining exclusive live output slice.
        // GRND_NONBLOCK (1) fails if the kernel pool is not initialized; it never
        // falls back to weak data or waits indefinitely for initialization.
        let count = unsafe {
            getrandom(
                output[written..].as_mut_ptr().cast(),
                output.len() - written,
                1,
            )
        };
        if count < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                && interruptions < 8
            {
                interruptions += 1;
                continue;
            }
            return Err(Error::Entropy);
        }
        if count == 0 || count as usize > output.len() - written {
            return Err(Error::Entropy);
        }
        written += count as usize;
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn fill(_: &mut [u8; 64]) -> Result<(), Error> {
    Err(Error::Unsupported)
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use crate::tls::ServerFlight;
    #[test]
    fn native_entropy_starts_distinct_offline_handshakes() {
        let first = ServerFlight::start("auth.openai.com").unwrap();
        let second = ServerFlight::start("auth.openai.com").unwrap();
        // Smoke coverage of the native boundary, not a statistical RNG audit.
        assert_ne!(
            first.client_hello().unwrap(),
            second.client_hello().unwrap()
        );
        assert!(!first.is_ready_for_verification());
        assert!(!second.is_ready_for_verification());
    }
}
