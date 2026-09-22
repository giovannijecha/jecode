//! Native Linux terminal boundary: termios, poll, ioctl. No terminal helper.
#![allow(unsafe_code)]
use super::super::{Key, input::Decoder};
use std::{
    io::{self, Read},
    time::Instant,
};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Termios {
    input: u32,
    output: u32,
    control: u32,
    local: u32,
    line: u8,
    characters: [u8; 32],
    input_speed: u32,
    output_speed: u32,
}
#[repr(C)]
struct Poll {
    fd: i32,
    events: i16,
    revents: i16,
}
#[repr(C)]
#[derive(Default)]
struct Size {
    rows: u16,
    columns: u16,
    x: u16,
    y: u16,
}
unsafe extern "C" {
    fn tcgetattr(fd: i32, value: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, action: i32, value: *const Termios) -> i32;
    fn poll(fds: *mut Poll, count: usize, timeout: i32) -> i32;
    fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
}
pub(in crate::terminal) struct Terminal {
    saved: Termios,
    decoder: Decoder,
}
impl Terminal {
    pub fn open() -> io::Result<Self> {
        let mut saved = Termios::default();
        // SAFETY: complete writable Linux ABI termios; fd 0 belongs to the caller.
        if unsafe { tcgetattr(0, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = saved;
        raw.input &= !(1 | 2 | 8 | 32 | 64 | 128 | 256 | 1024);
        raw.output &= !1;
        raw.control = (raw.control & !(0x30 | 0x100)) | 0x30;
        raw.local &= !(1 | 2 | 8 | 64 | 32768);
        raw.characters[6] = 1; // VMIN
        raw.characters[5] = 0; // VTIME
        // SAFETY: valid immutable layout; TCSANOW applies no queue flush.
        if unsafe { tcsetattr(0, 0, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            saved,
            decoder: Decoder::default(),
        })
    }
    pub fn size(&self) -> io::Result<(usize, usize)> {
        let mut size = Size::default();
        // SAFETY: TIOCGWINSZ writes exactly this four-u16 structure.
        if unsafe { ioctl(1, 0x5413, &mut size) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            usize::from(size.columns).max(1),
            usize::from(size.rows).max(1),
        ))
    }
    pub fn poll(&mut self) -> io::Result<Vec<Key>> {
        let mut fd = Poll {
            fd: 0,
            events: 1,
            revents: 0,
        };
        // SAFETY: one initialized pollfd is available for the duration of poll.
        let result = unsafe { poll(&mut fd, 1, 16) };
        if result < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(Vec::new())
            } else {
                Err(error)
            };
        }
        if fd.revents & (8 | 16 | 32) != 0 {
            return Err(io::Error::other("terminal disconnected"));
        }
        if fd.revents & 1 != 0 {
            let mut bytes = [0; 256];
            let count = io::stdin().read(&mut bytes)?;
            if count == 0 {
                return Err(io::Error::other("terminal input closed"));
            }
            return Ok(self.decoder.push(&bytes[..count], Instant::now()));
        }
        Ok(self.decoder.idle(Instant::now()))
    }
    pub fn diagnostic_position(&self) -> io::Result<String> {
        Ok("cursor=unavailable".into())
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: restore the saved attributes of the borrowed input descriptor.
        unsafe {
            tcsetattr(0, 0, &self.saved);
        }
    }
}
