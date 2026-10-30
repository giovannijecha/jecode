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
        #[cfg(any(windows, test))]
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
            #[cfg(any(windows, test))]
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
                    for event in self.flush() {
                        push(&mut ready, event);
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
        #[cfg(any(windows, test))]
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
mod tests {
    use super::*;

    #[test]
    fn accepts_bel_and_st_with_variable_component_widths() {
        assert!(matches!(
            candidate(b"\x1b]11;rgb:ffff/ffff/ffff\x07"),
            Match::Reply(true)
        ));
        assert!(matches!(
            candidate(b"\x1b]11;rgb:0/0/0\x1b\\"),
            Match::Reply(false)
        ));
        assert!(matches!(
            candidate(b"\x1b]11;rgb:eeee/eeee/eeee\x07"),
            Match::Reply(true)
        ));
    }

    #[test]
    fn malformed_and_unrelated_sequences_are_not_consumed() {
        assert!(matches!(candidate(b"\x1b]11;rgb:zz/0/0\x07"), Match::Other));
        assert!(matches!(
            candidate(b"\x1b]10;rgb:fff/fff/fff\x07"),
            Match::Other
        ));
        assert!(matches!(candidate(b"\x1b[A"), Match::Other));
    }

    #[test]
    fn split_reply_preserves_following_paste_and_unrelated_input() {
        let mut probe = Probe::for_test();
        let (events, light) =
            probe.feed(Input::Paste("\x1b]11;rgb:ffff/".encode_utf16().collect()));
        assert!(events.is_empty() && light.is_none());
        let (events, light) =
            probe.feed(Input::Paste("ffff/ffff\x07draft".encode_utf16().collect()));
        assert_eq!(light, Some(true));
        assert!(
            matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "draft")
        );
        let mut probe = Probe::for_test();
        let (events, light) = probe.feed(Input::Paste("\x1b[A1".encode_utf16().collect()));
        assert!(light.is_none());
        assert!(
            matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "\x1b[A1")
        );
    }

    #[test]
    fn a_stalled_reply_is_replayed_without_a_background_decision() {
        let mut probe = Probe::for_test();
        let (events, light) = probe.feed(Input::Paste("\x1b]11;rgb:ffff".encode_utf16().collect()));
        assert!(events.is_empty() && light.is_none());
        // The window closing does not cut a reply in progress.
        probe.started -= TIMEOUT;
        let (events, fallback) = probe.expire();
        assert!(events.is_empty() && fallback.is_none());
        probe.last_at = Some(Instant::now() - REPLY_GAP);
        let (events, fallback) = probe.expire();
        assert_eq!(fallback, None);
        assert!(
            matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "\x1b]11;rgb:ffff")
        );
    }

    fn paste(text: &str) -> Input {
        Input::Paste(text.encode_utf16().collect())
    }

    #[test]
    fn a_reply_slower_than_a_lone_escape_still_decides_the_background() {
        let mut probe = Probe::for_test();
        assert!(probe.feed(paste("\x1b]11;rgb:ff")).0.is_empty());
        // Longer than the lone-Escape wait, shorter than the reply gap.
        probe.last_at = Some(Instant::now() - PREFIX_TIMEOUT * 2);
        assert!(probe.expire().0.is_empty());
        let (events, light) = probe.feed(paste("ff/ffff/ffff\x07"));
        assert!(events.is_empty());
        assert_eq!(light, Some(true));
    }

    #[test]
    fn one_late_reply_is_consumed_after_the_window_and_later_input_passes() {
        let mut probe = Probe::for_test();
        probe.native_hint = Some(false);
        probe.started -= TIMEOUT;
        let (events, fallback) = probe.expire();
        assert!(events.is_empty());
        assert_eq!(fallback, Some(false));
        assert_eq!(probe.wait(), None, "no timer while nothing is held");
        // The late reply arrives split, then typing follows in the same event.
        assert!(probe.feed(paste("\x1b]11;rgb:0000/")).0.is_empty());
        let (events, light) = probe.feed(paste("0000/0000\x1b\\draft"));
        assert_eq!(light, None, "the settled palette stays");
        assert!(
            matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "draft")
        );
        // A second sequence is ordinary input again.
        let (events, _) = probe.feed(paste("\x1b]11;rgb:0/0/0\x07"));
        assert!(
            matches!(&events[..], [Input::Paste(units)] if String::from_utf16(units).unwrap() == "\x1b]11;rgb:0/0/0\x07")
        );
    }

    #[test]
    fn a_lone_escape_is_released_promptly_and_unicode_paste_is_unchanged() {
        let mut probe = Probe::for_test();
        let (events, light) = probe.feed(Input::Paste(vec![27]));
        assert!(events.is_empty() && light.is_none());
        probe.last_at = Some(Instant::now() - PREFIX_TIMEOUT);
        let (events, fallback) = probe.expire();
        // Released as the Escape key, which the decoder does not hold again.
        assert!(matches!(&events[..], [Input::Key(key)] if key.code == 27 && key.character == 0));
        assert!(fallback.is_none() && probe.active);
        let unicode: Vec<u16> = "β🙂".encode_utf16().collect();
        let (events, light) = probe.feed(Input::Paste(unicode.clone()));
        assert!(light.is_none());
        assert!(matches!(&events[..], [Input::Paste(units)] if units == &unicode));
    }

    #[test]
    fn no_osc_reply_uses_only_the_native_background_hint() {
        let mut probe = Probe::for_test();
        probe.native_hint = Some(true);
        probe.started -= TIMEOUT;
        let (events, light) = probe.expire();
        assert!(events.is_empty());
        assert_eq!(light, Some(true));
        assert!(!probe.active);
    }
}
