//! The OSC 11 background probe. During a short startup window a complete
//! reply decides the background; after it, one late reply is still consumed
//! for the rest of the session, so it never reaches the composer as Alt+],
//! text and Ctrl+G. Every other input event is replayed to the key decoder.

use super::terminal::Input;
use std::time::{Duration, Instant};

const QUERY: &str = "\x1b]11;?\x07";
const TIMEOUT: Duration = Duration::from_millis(800);
/// How long a lone Escape waits for the rest of a reply.
const PREFIX_TIMEOUT: Duration = Duration::from_millis(150);
/// The longest pause between bytes once a reply is recognized.
const REPLY_GAP: Duration = Duration::from_millis(1000);
const LIMIT: usize = 48;

pub(super) struct Probe {
    started: Instant,
    pending: Vec<Input>,
    bytes: Vec<u8>,
    /// When the last held byte arrived.
    last_at: Option<Instant>,
    /// The window in which a reply decides the background.
    active: bool,
    /// No reply has been consumed yet.
    watching: bool,
    native_hint: Option<bool>,
}

impl Probe {
    pub fn start() -> Result<Self, String> {
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(QUERY.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|error| format!("Could not query terminal background: {error}"))?;
        Ok(Self::new())
    }

    fn new() -> Self {
        Self {
            started: Instant::now(),
            pending: Vec::new(),
            bytes: Vec::new(),
            last_at: None,
            active: true,
            watching: true,
            native_hint: None,
        }
    }

    #[cfg(test)]
    fn for_test() -> Self {
        Self::new()
    }

    /// A lone Escape leaves at the decoder's pace; a recognized reply may
    /// pause longer between its bytes.
    fn gap(&self) -> Duration {
        if self.bytes.len() > 1 {
            REPLY_GAP
        } else {
            PREFIX_TIMEOUT
        }
    }

    pub fn wait(&self) -> Option<Duration> {
        let window = self
            .active
            .then(|| TIMEOUT.saturating_sub(self.started.elapsed()));
        let gap = self
            .last_at
            .map(|at| self.gap().saturating_sub(at.elapsed()));
        match (window, gap) {
            (Some(window), Some(gap)) => Some(window.min(gap)),
            (window, gap) => window.or(gap),
        }
    }

    /// Ends the window, with the native hint when no reply came, and
    /// releases held bytes whose reply stalled.
    pub fn expire(&mut self) -> (Vec<Input>, Option<bool>) {
        let mut hint = None;
        if self.active && self.started.elapsed() >= TIMEOUT {
            self.active = false;
            hint = self.native_hint.take();
        }
        let stalled = self.last_at.is_some_and(|at| at.elapsed() >= self.gap());
        (if stalled { self.release() } else { Vec::new() }, hint)
    }

    /// A lone Escape that waited here is already a key: the decoder must not
    /// hold it again.
    fn release(&mut self) -> Vec<Input> {
        if self.bytes == [27] {
            self.flush();
            return vec![Input::Key(super::terminal::Key {
                code: 27,
                modifiers: 0,
                character: 0,
            })];
        }
        self.flush()
    }

    /// Returns events to decode and whether the terminal reported a light base.
    pub fn feed(&mut self, event: Input) -> (Vec<Input>, Option<bool>) {
        #[cfg(windows)]
        let event = match event {
            Input::NativeBackground(red, green, blue) => {
                self.native_hint = Some(light_rgb(red, green, blue));
                return (Vec::new(), None);
            }
            other => other,
        };
        if !self.watching {
            return (vec![event], None);
        }
        let units: Vec<Input> = match event {
            #[cfg(unix)]
            Input::Bytes(bytes) => bytes
                .into_iter()
                .map(|byte| Input::Bytes(vec![byte]))
                .collect(),
            #[cfg(any(windows, test))]
            Input::Paste(units) => units
                .into_iter()
                .map(|unit| Input::Paste(vec![unit]))
                .collect(),
            Input::Key(key) if key.character != 0 => vec![Input::Key(key)],
            other => {
                let mut ready = self.flush();
                ready.push(other);
                return (ready, None);
            }
        };
        let mut ready = Vec::new();
        let mut light = None;
        for unit in units {
            let Some(byte) = character(&unit) else {
                for event in self.flush() {
                    push(&mut ready, event);
                }
                push(&mut ready, unit);
                continue;
            };
            if !self.watching {
                push(&mut ready, unit);
                continue;
            }
            self.last_at = Some(Instant::now());
            self.bytes.push(byte);
            self.pending.push(unit);
            match candidate(&self.bytes) {
                Match::More if self.bytes.len() < LIMIT => {}
                Match::Reply(value) => {
                    // A late reply is consumed without changing the palette
                    // the window already settled.
                    if self.active {
                        light = Some(value);
                        self.native_hint = None;
                    }
                    self.pending.clear();
                    self.bytes.clear();
                    self.last_at = None;
                    self.active = false;
                    self.watching = false;
                }
                _ => {
                    // The Escape that broke a match may open the reply itself, whose
                    // closing BEL would otherwise reach the composer as Ctrl+G.
                    let opener = (byte == 27 && self.bytes.len() > 1)
                        .then(|| self.pending.pop())
                        .flatten();
                    if opener.is_some() {
                        self.bytes.pop();
                    }
                    for event in self.release() {
                        push(&mut ready, event);
                    }
                    if let Some(opener) = opener {
                        self.bytes.push(byte);
                        self.pending.push(opener);
                        self.last_at = Some(Instant::now());
                    }
                }
            }
        }
        (ready, light)
    }

    fn flush(&mut self) -> Vec<Input> {
        self.bytes.clear();
        self.last_at = None;
        let pending = std::mem::take(&mut self.pending);
        let mut ready = Vec::new();
        for event in pending {
            push(&mut ready, event);
        }
        ready
    }
}

fn character(input: &Input) -> Option<u8> {
    let unit = match input {
        #[cfg(unix)]
        Input::Bytes(bytes) => u16::from(*bytes.first()?),
        #[cfg(any(windows, test))]
        Input::Paste(units) => *units.first()?,
        Input::Key(key) => key.character,
        _ => return None,
    };
    u8::try_from(unit).ok()
}

fn push(events: &mut Vec<Input>, event: Input) {
    match event {
        #[cfg(unix)]
        Input::Bytes(mut bytes) => {
            if let Some(Input::Bytes(previous)) = events.last_mut() {
                previous.append(&mut bytes);
            } else {
                events.push(Input::Bytes(bytes));
            }
        }
        #[cfg(any(windows, test))]
        Input::Paste(mut units) => {
            if let Some(Input::Paste(previous)) = events.last_mut() {
                previous.append(&mut units);
            } else {
                events.push(Input::Paste(units));
            }
        }
        other => events.push(other),
    }
}

enum Match {
    More,
    Reply(bool),
    Other,
}

fn candidate(bytes: &[u8]) -> Match {
    const PREFIX: &[u8] = b"\x1b]11;rgb:";
    if bytes.len() <= PREFIX.len() {
        return if PREFIX.starts_with(bytes) {
            Match::More
        } else {
            Match::Other
        };
    }
    if !bytes.starts_with(PREFIX) {
        return Match::Other;
    }
    let body = &bytes[PREFIX.len()..];
    let (body, finished) = if body.last() == Some(&7) {
        (&body[..body.len() - 1], true)
    } else if body.ends_with(b"\x1b\\") {
        (&body[..body.len() - 2], true)
    } else {
        (body, false)
    };
    if !finished && body.last() == Some(&27) {
        if body[..body.len() - 1]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() || *byte == b'/')
        {
            return Match::More;
        }
        return Match::Other;
    }
    let mut parts = body.split(|byte| *byte == b'/');
    let colors: Vec<_> = parts.by_ref().collect();
    if colors.len() > 3
        || colors
            .iter()
            .any(|part| part.len() > 4 || !part.iter().all(u8::is_ascii_hexdigit))
    {
        return Match::Other;
    }
    if !finished {
        return Match::More;
    }
    if colors.len() != 3 || colors.iter().any(|part| part.is_empty()) {
        return Match::Other;
    }
    let components: Vec<_> = colors
        .iter()
        .map(|part| {
            let value = u32::from_str_radix(std::str::from_utf8(part).ok()?, 16).ok()?;
            let max = (1u32 << (4 * part.len())) - 1;
            Some(value as f64 / max as f64)
        })
        .collect::<Option<Vec<_>>>()
        .unwrap();
    Match::Reply(light(components[0], components[1], components[2]))
}

#[cfg(windows)]
fn light_rgb(red: u8, green: u8, blue: u8) -> bool {
    light(
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
    )
}

fn light(red: f64, green: f64, blue: f64) -> bool {
    let linear = |value: f64| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue) > 0.5
}

#[cfg(test)]
mod tests;
