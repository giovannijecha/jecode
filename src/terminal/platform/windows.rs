//! Windows console input records and VT output, through kernel32 only.
#![allow(unsafe_code)]
use super::super::{Key, input::Decoder};
use std::{ffi::c_void, io, time::Instant};
type Handle = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Coord {
    x: i16,
    y: i16,
}
#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
}
#[repr(C)]
#[derive(Default)]
struct Info {
    size: Coord,
    cursor: Coord,
    attributes: u16,
    window: Rect,
    max: Coord,
}
#[repr(C)]
#[derive(Default)]
struct Record {
    kind: u16,
    padding: u16,
    // KEY_EVENT_RECORD is 16 bytes, union alignment 4.
    down: i32,
    repeat: u16,
    key: u16,
    scan: u16,
    character: u16,
    modifiers: u32,
}
fn control_key(virtual_key: u16) -> Option<Key> {
    match virtual_key {
        0x51 => Some(Key::Quit),
        0x43 => Some(Key::Interrupt),
        0x41 => Some(Key::Home),
        0x45 => Some(Key::End),
        0x4a | 0x4f | 0x0d => Some(Key::Newline),
        0x50 => Some(Key::HistoryPrevious),
        0x4e => Some(Key::HistoryNext),
        0x57 | 0x08 => Some(Key::WordBackspace),
        0x25 => Some(Key::WordLeft),
        0x27 => Some(Key::WordRight),
        0x2e => Some(Key::WordDelete),
        0x24 => Some(Key::DraftStart),
        0x23 => Some(Key::DraftEnd),
        _ => None,
    }
}
fn modified_backspace(record: &Record, control_down: bool) -> bool {
    record.modifiers & 3 == 0
        && matches!(record.key, 0 | 0x08)
        && record.character == 8
        && (record.modifiers & 12 != 0 || control_down)
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(id: u32) -> Handle;
    fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
    fn SetConsoleMode(handle: Handle, mode: u32) -> i32;
    fn GetConsoleScreenBufferInfo(handle: Handle, info: *mut Info) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn ReadConsoleInputW(handle: Handle, record: *mut Record, length: u32, read: *mut u32) -> i32;
}
pub(in crate::terminal) struct Terminal {
    input: Handle,
    output: Handle,
    input_mode: u32,
    output_mode: u32,
    high: Option<u16>,
    control_down: bool,
    decoder: Decoder,
}
impl Terminal {
    pub fn open() -> io::Result<Self> {
        // SAFETY: borrowed process standard handles, never closed by this guard.
        let (input, output) = unsafe { (GetStdHandle(-10i32 as u32), GetStdHandle(-11i32 as u32)) };
        let (mut input_mode, mut output_mode) = (0, 0);
        // SAFETY: valid local out parameters; invalid handles return failure.
        if unsafe {
            GetConsoleMode(input, &mut input_mode) == 0
                || GetConsoleMode(output, &mut output_mode) == 0
        } {
            return Err(io::Error::last_os_error());
        }
        let terminal = Self {
            input,
            output,
            input_mode,
            output_mode,
            high: None,
            control_down: false,
            decoder: Decoder::default(),
        };
        // Guard is established before either mutation, including partial failure.
        // Disable line/echo/processed input and quick edit. Preserve VT paste
        // delimiters through ConPTY; key records feed the bounded VT decoder.
        // SAFETY: console handles validated above, documented console flag masks.
        if unsafe {
            SetConsoleMode(
                input,
                (input_mode & !(1 | 2 | 4 | 0x10 | 0x40)) | 0x80 | 8 | 0x200,
            ) == 0
                || SetConsoleMode(output, output_mode | 1 | 4 | 8) == 0
        } {
            return Err(io::Error::last_os_error());
        }
        Ok(terminal)
    }
    pub fn size(&self) -> io::Result<(usize, usize)> {
        let mut info = Info::default();
        // SAFETY: ABI-sized console information out parameter.
        if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            (i32::from(info.window.right) - i32::from(info.window.left) + 1).max(1) as usize,
            (i32::from(info.window.bottom) - i32::from(info.window.top) + 1).max(1) as usize,
        ))
    }
    pub fn poll(&mut self) -> io::Result<Vec<Key>> {
        // SAFETY: borrowed input handle remains open throughout this call.
        match unsafe { WaitForSingleObject(self.input, 16) } {
            258 => return Ok(self.decoder.idle(Instant::now())),
            0 => {}
            _ => return Err(io::Error::last_os_error()),
        }
        let mut record = Record::default();
        let mut count = 0;
        // SAFETY: one full INPUT_RECORD with correct size/alignment, readable input handle.
        if unsafe { ReadConsoleInputW(self.input, &mut record, 1, &mut count) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if count != 1 || record.kind != 1 {
            return Ok(Vec::new());
        }
        if matches!(record.key, 0x11 | 0xa2 | 0xa3) {
            // ConPTY can synthesize a Ctrl key-down before a BS character,
            // then put no modifier on the character's own key record.
            self.control_down = record.down != 0 || record.modifiers & 12 != 0;
            return Ok(Vec::new());
        }
        if record.down == 0 {
            return Ok(Vec::new());
        }
        let mut keys = Vec::new();
        for _ in 0..record.repeat.min(32) {
            let control = record.modifiers & 12 != 0;
            let alt = record.modifiers & 3 != 0;
            let shift = record.modifiers & 0x10 != 0;
            let key = if modified_backspace(&record, self.control_down) {
                Some(Key::WordBackspace)
            } else if control && !alt && record.key != 0 {
                control_key(record.key)
            } else if shift && !alt && record.key == 0x0d {
                Some(Key::Newline)
            } else {
                match record.key {
                    0x0d => Some(Key::Enter),
                    0x09 => Some(Key::Tab),
                    0x1b => Some(Key::Escape),
                    0x08 => Some(Key::Backspace),
                    0x25 => Some(Key::Left),
                    0x26 => Some(Key::Up),
                    0x28 => Some(Key::Down),
                    0x27 => Some(Key::Right),
                    0x24 => Some(Key::Home),
                    0x23 => Some(Key::End),
                    0x2e => Some(Key::Delete),
                    0x21 => Some(Key::PageUp),
                    0x22 => Some(Key::PageDown),
                    _ => {
                        let unit = record.character;
                        if (0xd800..=0xdbff).contains(&unit) {
                            self.high = Some(unit);
                            None
                        } else if (0xdc00..=0xdfff).contains(&unit) {
                            self.high
                                .take()
                                .and_then(|high| {
                                    char::from_u32(
                                        0x10000
                                            + ((u32::from(high) - 0xd800) << 10)
                                            + u32::from(unit)
                                            - 0xdc00,
                                    )
                                })
                                .map(|ch| Key::Text(ch.to_string()))
                        } else {
                            self.high = None;
                            char::from_u32(u32::from(unit))
                                .filter(|ch| *ch != '\0')
                                .map(|ch| Key::Text(ch.to_string()))
                        }
                    }
                }
            };
            if let Some(key) = key {
                self.high = None;
                keys.extend(self.decoder.native(key, Instant::now()));
            }
        }
        Ok(keys)
    }
    pub fn diagnostic_position(&self) -> io::Result<String> {
        let mut info = Info::default();
        // SAFETY: owned writable ABI structure and borrowed live output handle.
        if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(format!(
            "buffer={}x{} cursor={},{} viewport={},{},{},{}",
            info.size.x,
            info.size.y,
            info.cursor.x,
            info.cursor.y,
            info.window.left,
            info.window.top,
            info.window.right,
            info.window.bottom
        ))
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: restore both borrowed handles; failures cannot panic during unwind.
        unsafe {
            SetConsoleMode(self.input, self.input_mode);
            SetConsoleMode(self.output, self.output_mode);
        }
    }
}

#[cfg(test)]
#[test]
fn windows_console_abi_layouts() {
    assert_eq!(std::mem::size_of::<Record>(), 20);
    assert_eq!(std::mem::align_of::<Record>(), 4);
    assert_eq!(std::mem::size_of::<Info>(), 22);
}
#[cfg(test)]
#[test]
fn native_control_records_distinguish_editor_actions_from_exit_and_submit() {
    assert_eq!(control_key(0x51), Some(Key::Quit));
    assert_eq!(control_key(0x43), Some(Key::Interrupt));
    assert_eq!(control_key(0x4f), Some(Key::Newline));
    assert_eq!(control_key(0x0d), Some(Key::Newline));
    assert_eq!(control_key(0x50), Some(Key::HistoryPrevious));
    assert_eq!(control_key(0x4e), Some(Key::HistoryNext));
    assert_eq!(control_key(0x08), Some(Key::WordBackspace));
    assert_eq!(control_key(0x2e), Some(Key::WordDelete));
    assert_eq!(control_key(0x25), Some(Key::WordLeft));
    assert_eq!(control_key(0x27), Some(Key::WordRight));
    assert_eq!(control_key(0x24), Some(Key::DraftStart));
    assert_eq!(control_key(0x23), Some(Key::DraftEnd));
}

#[cfg(test)]
#[test]
fn synthesized_ctrl_backspace_requires_a_distinguishable_ctrl_record() {
    let mut record = Record {
        kind: 1,
        down: 1,
        key: 0,
        character: 8,
        ..Record::default()
    };
    assert!(!modified_backspace(&record, false));
    assert!(modified_backspace(&record, true));
    record.modifiers = 8;
    assert!(modified_backspace(&record, false));
    record.modifiers = 2;
    assert!(!modified_backspace(&record, true));
    record.modifiers = 0;
    record.character = 127;
    assert!(!modified_backspace(&record, true));
    record.character = 8;
    record.key = 0x57;
    assert!(!modified_backspace(&record, true));
}
