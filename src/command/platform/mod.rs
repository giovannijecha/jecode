#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::Process;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(super) use linux::Process;

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) struct Process;
#[cfg(not(any(windows, target_os = "linux")))]
impl Process {
    pub fn spawn(_: &str, _: &crate::workspace::Directory) -> std::io::Result<Self> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    pub fn read(&mut self, _: super::Channel, _: &mut [u8]) -> std::io::Result<usize> {
        unreachable!()
    }
    pub fn exited(&mut self) -> std::io::Result<bool> {
        unreachable!()
    }
    pub fn finish(&mut self) -> std::io::Result<super::Exit> {
        unreachable!()
    }
}
