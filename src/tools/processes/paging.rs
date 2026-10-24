//! Bounded model output windows over complete saved process streams.
use crate::json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) enum OutputMode {
    Next,
    Tail,
}

impl OutputMode {
    pub(super) fn parse(arguments: &Value) -> Result<Self, String> {
        match arguments.get("output_mode") {
            None => Ok(Self::Next),
            Some(Value::String(mode)) if mode == "next" => Ok(Self::Next),
            Some(Value::String(mode)) if mode == "tail" => Ok(Self::Tail),
            _ => Err("output_mode must be next or tail".into()),
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Tail => "tail",
        }
    }
}

pub(super) fn read_output(
    path: &Path,
    from: u64,
    finished: bool,
    limit: usize,
    mode: OutputMode,
) -> std::io::Result<Chunk> {
    match mode {
        OutputMode::Next => read_after(path, from, finished, limit),
        OutputMode::Tail => read_tail(path, from, finished, limit),
    }
}

pub(super) struct Chunk {
    pub(super) text: String,
    pub(super) offset: u64,
    pub(super) next: u64,
    pub(super) length: u64,
}

/// Reads the next bounded window without advancing past unread bytes.
/// A trailing incomplete UTF-8 sequence waits while the process runs.
pub(super) fn read_after(
    path: &Path,
    from: u64,
    finished: bool,
    limit: usize,
) -> std::io::Result<Chunk> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len().max(from);
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    file.take((length - from).min(limit.saturating_add(3) as u64))
        .read_to_end(&mut bytes)?;
    let target = bytes.len().min(limit);
    let mut end = complete_prefix(&bytes[..target]);
    if end == 0 && bytes.len() > target {
        // A one-byte budget must still make progress on a complete UTF-8 character.
        let width = match bytes[0] {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if bytes.len() >= width {
            end = width;
        }
    }
    if finished && bytes.len() <= limit {
        end = bytes.len();
    }
    Ok(Chunk {
        text: String::from_utf8_lossy(&bytes[..end]).into_owned(),
        offset: from,
        next: from + end as u64,
        length,
    })
}

fn read_tail(path: &Path, from: u64, finished: bool, limit: usize) -> std::io::Result<Chunk> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len().max(from);
    let start = length.saturating_sub(limit as u64).max(from);
    // Include the preceding UTF-8 bytes when the requested tail begins inside
    // a character; a tiny budget can still show one complete character.
    let context = start.saturating_sub(3).max(from);
    file.seek(SeekFrom::Start(context))?;
    let mut bytes = Vec::new();
    file.take(length - context).read_to_end(&mut bytes)?;
    let mut start = (start - context) as usize;
    while start > 0 && bytes.get(start).is_some_and(|byte| byte & 0xC0 == 0x80) {
        start -= 1;
    }
    let end = if finished {
        bytes.len()
    } else {
        complete_prefix(&bytes)
    };
    let start = start.min(end);
    Ok(Chunk {
        text: String::from_utf8_lossy(&bytes[start..end]).into_owned(),
        offset: context + start as u64,
        next: context + end as u64,
        length,
    })
}

pub(super) fn complete_prefix(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(4) {
        let index = bytes.len() - back;
        let width = match bytes[index] {
            0x80..=0xBF => continue,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        return if back < width { index } else { bytes.len() };
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn tiny_tail_keeps_a_complete_character_and_never_replays_a_cursor() {
        let directory = Directory::new();
        let path = directory.path().join("stream");
        std::fs::write(&path, "prefix🙂").unwrap();
        let tail = read_tail(&path, 2, true, 1).unwrap();
        assert_eq!(tail.text, "🙂");
        assert_eq!((tail.offset, tail.next), (6, 10));
        let drained = read_tail(&path, tail.next, true, 1).unwrap();
        assert_eq!(drained.text, "");
        assert_eq!((drained.offset, drained.next), (10, 10));
    }

    #[test]
    fn a_live_tail_waits_for_the_rest_of_a_split_character() {
        let directory = Directory::new();
        let path = directory.path().join("stream");
        let text = "x🙂";
        std::fs::write(&path, &text.as_bytes()[..3]).unwrap();
        let partial = read_tail(&path, 0, false, 1).unwrap();
        assert_eq!(partial.text, "");
        assert_eq!((partial.offset, partial.next, partial.length), (1, 1, 3));
        std::fs::write(&path, text).unwrap();
        let complete = read_tail(&path, partial.next, false, 1).unwrap();
        assert_eq!(complete.text, "🙂");
        assert_eq!((complete.offset, complete.next), (1, 5));
    }

    #[test]
    fn output_mode_defaults_to_next_and_rejects_unknown_values() {
        assert_eq!(
            OutputMode::parse(&Value::object([])).unwrap().name(),
            "next"
        );
        for value in [Value::string("unknown"), Value::number(1), Value::Null] {
            assert!(OutputMode::parse(&Value::object([("output_mode", value)])).is_err());
        }
    }
}
