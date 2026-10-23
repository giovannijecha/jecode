//! One versioned pack: fixed header, raw file bytes, checked metadata, footer.
use super::files::Mode;
use super::hash::{self, Digest};
use super::state::State;
use super::types::Snapshot;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

const MAGIC: &[u8; 8] = b"JECRCP04";
const END: &[u8; 8] = b"RCPEND04";
pub const HEADER: u64 = 32;
const FOOTER: u64 = 24;

pub fn header(file: &mut File, created: u64) -> Result<(), String> {
    file.write_all(MAGIC)
        .and_then(|_| file.write_all(&created.to_le_bytes()))
        .and_then(|_| file.write_all(&[0; 16]))
        .map_err(|e| format!("Could not write recovery header: {e}"))
}

pub fn finish(file: &mut File, metadata: &[u8]) -> Result<(), String> {
    let checksum = hash::checksum(&[MAGIC, metadata]);
    file.write_all(metadata)
        .and_then(|_| file.write_all(&(metadata.len() as u64).to_le_bytes()))
        .and_then(|_| file.write_all(&checksum.to_le_bytes()))
        .and_then(|_| file.write_all(END))
        .map_err(|e| format!("Could not finish recovery pack: {e}"))
}

pub fn read(file: &mut File) -> Result<Snapshot, String> {
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size < HEADER + FOOTER {
        return Err("Truncated recovery pack".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut head = [0u8; HEADER as usize];
    file.read_exact(&mut head).map_err(|e| e.to_string())?;
    let (version, end) = match &head[..8] {
        b"JECRCP02" => (2, b"RCPEND02"),
        b"JECRCP03" => (3, b"RCPEND03"),
        b"JECRCP04" => (4, END),
        _ => return Err("Invalid recovery header".into()),
    };
    if head[16..].iter().any(|b| *b != 0) {
        return Err("Invalid recovery header".into());
    }
    let created = u64::from_le_bytes(head[8..16].try_into().unwrap());
    file.seek(SeekFrom::End(-(FOOTER as i64)))
        .map_err(|e| e.to_string())?;
    let mut foot = [0u8; FOOTER as usize];
    file.read_exact(&mut foot).map_err(|e| e.to_string())?;
    if &foot[16..] != end {
        return Err("Invalid recovery footer".into());
    }
    let length = u64::from_le_bytes(foot[..8].try_into().unwrap());
    let checksum = u64::from_le_bytes(foot[8..16].try_into().unwrap());
    if length > size - HEADER - FOOTER {
        return Err("Invalid recovery metadata length".into());
    }
    let start = size - FOOTER - length;
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let allocation =
        usize::try_from(length).map_err(|_| "Recovery metadata exceeds platform address space")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(allocation)
        .map_err(|_| "Could not allocate recovery metadata")?;
    bytes.resize(allocation, 0);
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    if hash::checksum(&[&head[..8], &bytes]) != checksum {
        return Err("Corrupt recovery metadata".into());
    }
    let snapshot = decode(&bytes, version)?;
    if snapshot.created != created {
        return Err("Recovery creation time mismatch".into());
    }
    let mut next = HEADER;
    for state in snapshot.entries.values() {
        if let State::File { offset, length, .. } = state {
            if *offset != next {
                return Err("Recovery payload offsets are inconsistent".into());
            }
            next = offset
                .checked_add(*length)
                .filter(|end| *end <= start)
                .ok_or("Recovery payload extends beyond pack")?;
        }
    }
    if next != start {
        return Err("Recovery pack has unreferenced payload bytes".into());
    }
    Ok(snapshot)
}

pub(super) fn encode(snapshot: &Snapshot) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    put_text(&mut out, &snapshot.reference)?;
    put_u64(&mut out, snapshot.created);
    put_text(&mut out, &snapshot.tool)?;
    put_text(&mut out, &snapshot.origin)?;
    put_count(&mut out, snapshot.entries.len())?;
    for (path, state) in &snapshot.entries {
        put_text(&mut out, path)?;
        match state {
            State::Absent => out.push(0),
            State::File {
                offset,
                length,
                digest,
                mode,
            } => {
                out.push(1);
                put_u64(&mut out, *offset);
                put_u64(&mut out, *length);
                put_u64(&mut out, digest.0[0]);
                put_u64(&mut out, digest.0[1]);
                put_u32(&mut out, mode.0);
            }
            State::Directory { mode } => {
                out.push(2);
                put_u32(&mut out, mode.0);
            }
            State::Link { target, directory } => {
                out.push(3);
                put_text(&mut out, target)?;
                out.push(u8::from(*directory));
            }
            State::Unreadable { reason } => {
                out.push(4);
                put_text(&mut out, reason)?;
            }
        }
    }
    for strings in [&snapshot.roots, &snapshot.excluded, &snapshot.ignored] {
        put_count(&mut out, strings.len())?;
        for string in strings {
            put_text(&mut out, string)?;
        }
    }
    put_count(&mut out, snapshot.ignore_rules.local.len())?;
    for (directory, text) in &snapshot.ignore_rules.local {
        put_text(&mut out, directory)?;
        put_text(&mut out, text)?;
    }
    put_count(&mut out, snapshot.ignore_rules.outer.len())?;
    for (prefix, text) in &snapshot.ignore_rules.outer {
        put_text(&mut out, prefix)?;
        put_text(&mut out, text)?;
    }
    put_count(&mut out, snapshot.partial.len())?;
    for (path, reason) in &snapshot.partial {
        put_text(&mut out, path)?;
        put_text(&mut out, reason)?;
    }
    Ok(out)
}

fn decode(bytes: &[u8], version: u8) -> Result<Snapshot, String> {
    let mut input = Input { bytes, at: 0 };
    let reference = input.text()?;
    let created = input.u64()?;
    let tool = input.text()?;
    let origin = input.text()?;
    let count = input.count()?;
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        let path = input.text()?;
        if !valid_path(&path, false) {
            return Err("Invalid recovery entry path".into());
        }
        let state = match input.u8()? {
            0 => State::Absent,
            1 => State::File {
                offset: input.u64()?,
                length: input.u64()?,
                digest: Digest([input.u64()?, input.u64()?]),
                mode: Mode(input.u32()?),
            },
            2 => State::Directory {
                mode: Mode(input.u32()?),
            },
            3 => State::Link {
                target: input.text()?,
                directory: match input.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err("Invalid recovery link kind".into()),
                },
            },
            4 => State::Unreadable {
                reason: input.text()?,
            },
            _ => return Err("Unknown recovery state".into()),
        };
        if entries.insert(path, state).is_some() {
            return Err("Duplicate recovery path".into());
        }
    }
    let roots = input.strings()?;
    let excluded = input.strings()?;
    let ignored = if version >= 3 {
        input.strings()?
    } else {
        Vec::new()
    };
    let mut ignore_rules = super::paths::IgnoreRules::default();
    if version >= 4 {
        for _ in 0..input.count()? {
            let directory = input.text()?;
            let text = input.text()?;
            if !valid_path(&directory, true) || ignore_rules.local.insert(directory, text).is_some()
            {
                return Err("Invalid recovery ignore directory".into());
            }
        }
        for _ in 0..input.count()? {
            let prefix = input.text()?;
            let text = input.text()?;
            if !valid_path(&prefix, true) {
                return Err("Invalid recovery ignore prefix".into());
            }
            ignore_rules.outer.push((prefix, text));
        }
    }
    if roots.iter().any(|root| !valid_path(root, true))
        || roots.windows(2).any(|pair| pair[0] >= pair[1])
        || excluded
            .iter()
            .chain(&ignored)
            .any(|path| !valid_path(path, false))
    {
        return Err("Invalid recovery scope path".into());
    }
    let count = input.count()?;
    let mut partial = Vec::new();
    for _ in 0..count {
        partial.push((input.text()?, input.text()?));
    }
    if input.at != bytes.len() {
        return Err("Trailing recovery metadata".into());
    }
    Ok(Snapshot {
        reference,
        created,
        tool,
        origin,
        entries,
        roots,
        excluded,
        ignored,
        ignore_rules,
        partial,
    })
}

fn valid_path(path: &str, allow_root: bool) -> bool {
    if path.is_empty() {
        return allow_root;
    }
    path.split('/').all(super::files::normal_component)
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_count(out: &mut Vec<u8>, value: usize) -> Result<(), String> {
    put_u32(
        out,
        u32::try_from(value).map_err(|_| "Too many recovery entries")?,
    );
    Ok(())
}
fn put_text(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    put_count(out, value.len())?;
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Input<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or("Truncated recovery metadata")?;
        let result = &self.bytes[self.at..end];
        self.at = end;
        Ok(result)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, String> {
        let count = self.u32()? as usize;
        if count > self.bytes.len().saturating_sub(self.at) {
            return Err("Invalid recovery count".into());
        }
        Ok(count)
    }
    fn text(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        String::from_utf8(self.take(length)?.to_vec()).map_err(|_| "Invalid recovery text".into())
    }
    fn strings(&mut self) -> Result<Vec<String>, String> {
        let count = self.count()?;
        let mut result = Vec::new();
        for _ in 0..count {
            result.push(self.text()?);
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "format_tests.rs"]
mod tests;
