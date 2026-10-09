#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::Terminal;
#[cfg(windows)]
pub use windows::Terminal;

#[derive(Debug, Clone, Copy)]
pub struct Key {
    pub code: u16,
    pub modifiers: u8,
    pub character: u16,
}
impl Key {
    pub fn ctrl(self) -> bool {
        self.modifiers & 4 != 0
    }
    pub fn shift(self) -> bool {
        self.modifiers & 2 != 0
    }
    pub fn alt(self) -> bool {
        self.modifiers & 1 != 0
    }
}

#[derive(Debug)]
pub enum Input {
    #[cfg(any(windows, test))]
    Key(Key),
    #[cfg(any(windows, test))]
    Paste(Vec<u16>),
    #[cfg(any(windows, test))]
    Scroll(i16),
    #[cfg(windows)]
    Modes(u32, u32, u32),
    #[cfg(unix)]
    Bytes(Vec<u8>),
    Size(Geometry),
    Error(String),
}

pub(super) const LEAVE: &str =
    "\x1b[0m\x1b[?2026l\x1b[?1006l\x1b[?1000l\x1b[?2004l\x1b[?1049l\x1b[?25h";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: usize,
    pub height: usize,
    pub row: usize,
    pub column: usize,
}
