//! Local terminal input; bytes go only to the live PTY control channel.

use super::super::{App, terminal::Key, view};

impl App {
    pub(in crate::tui) fn direct_input_active(&self) -> bool {
        self.state
            .processes
            .as_ref()
            .and_then(|panel| panel.logs.as_ref())
            .is_some_and(|logs| logs.input_mode)
    }

    pub(in crate::tui) fn process_text(&mut self, text: &str, pasted: bool) {
        let Some(panel) = self.state.processes.as_ref() else {
            return;
        };
        let Some(logs) = panel.logs.as_ref() else {
            return;
        };
        if logs.input_mode {
            self.send_direct(text.as_bytes());
            return;
        }
        if pasted
            || text != "i"
            || panel.confirm_stop.is_some()
            || !logs.interactive
            || !logs.process.as_ref().is_some_and(|p| p.active())
        {
            return;
        }
        let logs = self
            .state
            .processes
            .as_mut()
            .unwrap()
            .logs
            .as_mut()
            .unwrap();
        logs.input_mode = true;
        logs.stream = 0;
        logs.views[0].follow = true;
        if let Some((id, size)) = self.direct_resize_target()
            && let Err(error) = self.process_observer.resize_direct(&id, size)
        {
            self.leave_direct_input(&error);
        }
    }

    pub(super) fn direct_key(&mut self, key: Key) {
        if key.character == 29 || key.ctrl() && key.code == 221 {
            if let Some(logs) = self.state.processes.as_mut().and_then(|p| p.logs.as_mut()) {
                logs.input_mode = false;
            }
            return;
        }
        if let Some(bytes) = key_bytes(key) {
            self.send_direct(&bytes);
        }
    }

    pub(super) fn direct_resize_target(&self) -> Option<(String, (u16, u16))> {
        let logs = self.state.processes.as_ref()?.logs.as_ref()?;
        if !logs.input_mode {
            return None;
        }
        let cols = view::process_width(&self.state).clamp(1, 1000) as u16;
        let rows = view::process_log_rows(&self.state).1.clamp(1, 1000) as u16;
        Some((logs.id.clone(), (cols, rows)))
    }

    pub(super) fn leave_direct_input(&mut self, error: &str) {
        if let Some(logs) = self.state.processes.as_mut().and_then(|p| p.logs.as_mut()) {
            logs.input_mode = false;
        }
        self.warn(error);
    }

    fn send_direct(&mut self, bytes: &[u8]) {
        let Some(id) = self
            .state
            .processes
            .as_ref()
            .and_then(|p| p.logs.as_ref())
            .map(|l| l.id.clone())
        else {
            return;
        };
        if let Err(error) = self.process_observer.write_direct(&id, bytes) {
            self.leave_direct_input(&error);
        }
    }
}

fn key_bytes(key: Key) -> Option<Vec<u8>> {
    let code = key.code;
    let modifiers = 1 + u8::from(key.shift()) + 2 * u8::from(key.alt()) + 4 * u8::from(key.ctrl());
    let sequence = match code {
        37 => Some(("D", "1")),
        38 => Some(("A", "1")),
        39 => Some(("C", "1")),
        40 => Some(("B", "1")),
        36 => Some(("H", "1")),
        35 => Some(("F", "1")),
        45 => Some(("~", "2")),
        46 => Some(("~", "3")),
        33 => Some(("~", "5")),
        34 => Some(("~", "6")),
        _ => None,
    };
    if let Some((suffix, parameter)) = sequence {
        return Some(if modifiers == 1 {
            if suffix == "~" {
                format!("\x1b[{parameter}{suffix}")
            } else {
                format!("\x1b[{suffix}")
            }
            .into_bytes()
        } else {
            format!("\x1b[{parameter};{modifiers}{suffix}").into_bytes()
        });
    }
    if code == 9 && key.shift() && !key.ctrl() && !key.alt() {
        return Some(b"\x1b[Z".to_vec());
    }
    let byte = match code {
        13 => b'\r',
        9 => b'\t',
        8 => 0x7f,
        27 => 0x1b,
        65..=90 if key.ctrl() => (code - 64) as u8,
        32 if key.ctrl() => 0,
        220 if key.ctrl() => 28,
        221 if key.ctrl() => 29,
        54 if key.ctrl() => 30,
        189 if key.ctrl() => 31,
        _ if (1..=31).contains(&key.character) => key.character as u8,
        _ if key.character == 127 => 0x7f,
        _ => {
            if key.character == 0 {
                return None;
            }
            let character = char::from_u32(key.character.into())?;
            let mut bytes = character.to_string().into_bytes();
            if key.alt() {
                bytes.insert(0, 0x1b);
            }
            return Some(bytes);
        }
    };
    let mut bytes = vec![byte];
    if key.alt() && code != 27 {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u16, modifiers: u8, character: u16) -> Key {
        Key {
            code,
            modifiers,
            character,
        }
    }

    #[test]
    fn direct_keys_preserve_terminal_control_and_modifier_sequences() {
        assert_eq!(key_bytes(key(67, 4, 3)), Some(vec![3]));
        assert_eq!(key_bytes(key(13, 0, 13)), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(key(8, 0, 8)), Some(vec![127]));
        assert_eq!(key_bytes(key(38, 0, 0)), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(key(37, 4, 0)), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(key_bytes(key(9, 2, 9)), Some(b"\x1b[Z".to_vec()));
        assert_eq!(key_bytes(key(86, 1, b'v' as u16)), Some(b"\x1bv".to_vec()));
    }
}
