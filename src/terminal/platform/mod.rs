#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::Terminal;

// The termios layout/constants below are the Linux x86_64/aarch64 ABI.
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) use linux::Terminal;

#[cfg(not(any(
    windows,
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
pub(super) struct Terminal;
#[cfg(not(any(
    windows,
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
impl Terminal {
    pub fn open() -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "terminal preview supports Windows and Linux",
        ))
    }
    pub fn size(&self) -> std::io::Result<(usize, usize)> {
        unreachable!()
    }
    pub fn poll(&mut self) -> std::io::Result<Vec<super::Key>> {
        unreachable!()
    }
    pub fn diagnostic_position(&self) -> std::io::Result<String> {
        unreachable!()
    }
}
