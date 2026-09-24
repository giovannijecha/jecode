//! Bounded frames. Only the atomically replaced head commits a log prefix.
use crate::{json, state::Store};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
};

pub(super) const EVENT_LIMIT: usize = 80 * 1024 * 1024;
const HEADER: usize = 20;
const CHUNK: usize = 64 * 1024;
pub(super) const HASH_START: u64 = 0xcbf29ce484222325;

fn hash(mut value: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100000001b3);
    }
    value
}

pub(super) fn fingerprint(value: &json::Value) -> io::Result<u64> {
    let encoded = json::encode(value, EVENT_LIMIT)
        .map_err(|_| io::Error::other("one canonical item exceeds its bounded parser"))?;
    Ok(hash(HASH_START, encoded.as_bytes()))
}

pub(super) fn open(store: &Store, id: &str, create: bool) -> io::Result<File> {
    store.data_file(&format!("{id}.log"), !create)
}

pub(super) fn append(
    file: &mut File,
    turn: usize,
    value: &json::Value,
    offset: &mut u64,
    rolling: &mut u64,
) -> io::Result<()> {
    let bytes = json::encode(value, EVENT_LIMIT)
        .map_err(|_| io::Error::other("one canonical log event exceeds its bounded parser"))?;
    let length = u32::try_from(bytes.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let turn = u64::try_from(turn).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut header = [0u8; HEADER];
    header[..4].copy_from_slice(&length.to_le_bytes());
    header[4..12].copy_from_slice(&turn.to_le_bytes());
    header[12..20].copy_from_slice(&hash(HASH_START, bytes.as_bytes()).to_le_bytes());
    file.write_all(&header)?;
    for chunk in bytes.as_bytes().chunks(CHUNK) {
        file.write_all(chunk)?;
    }
    *offset = offset
        .checked_add(HEADER as u64 + u64::from(length))
        .ok_or(io::ErrorKind::InvalidData)?;
    *rolling = hash(hash(*rolling, &header), bytes.as_bytes());
    Ok(())
}

pub(super) fn prepare(file: &mut File, committed: u64) -> io::Result<()> {
    if file.metadata()?.len() < committed {
        return Err(corrupt());
    }
    file.set_len(committed)?;
    file.seek(SeekFrom::Start(committed))?;
    Ok(())
}

/// Read every committed byte, parsing only events at/after `first_turn`.
/// Bytes after `committed` are an incomplete, uncommitted tail.
pub(super) fn visit(
    store: &Store,
    id: &str,
    committed: u64,
    expected_hash: u64,
    first_turn: usize,
    last_turn: usize,
    mut event: impl FnMut(usize, json::Value) -> io::Result<()>,
) -> io::Result<()> {
    let mut file = store.read_file(&format!("{id}.log"))?;
    if file.metadata()?.len() < committed {
        return Err(corrupt());
    }
    let mut position = 0u64;
    let mut rolling = HASH_START;
    while position < committed {
        if committed - position < HEADER as u64 {
            return Err(corrupt());
        }
        let mut header = [0u8; HEADER];
        file.read_exact(&mut header).map_err(|_| corrupt())?;
        let length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        let turn = u64::from_le_bytes(header[4..12].try_into().unwrap());
        let checksum = u64::from_le_bytes(header[12..20].try_into().unwrap());
        if length > EVENT_LIMIT || committed - position - (HEADER as u64) < length as u64 {
            return Err(corrupt());
        }
        let keep = turn >= first_turn as u64 && turn < last_turn as u64;
        let mut bytes = keep.then(|| Vec::with_capacity(length));
        let mut remaining = length;
        let mut digest = HASH_START;
        rolling = hash(rolling, &header);
        let mut chunk = [0u8; CHUNK];
        while remaining != 0 {
            let count = remaining.min(CHUNK);
            file.read_exact(&mut chunk[..count])
                .map_err(|_| corrupt())?;
            digest = hash(digest, &chunk[..count]);
            rolling = hash(rolling, &chunk[..count]);
            if let Some(bytes) = &mut bytes {
                bytes.extend_from_slice(&chunk[..count]);
            }
            remaining -= count;
        }
        if digest != checksum {
            return Err(corrupt());
        }
        if let Some(bytes) = bytes {
            let source = std::str::from_utf8(&bytes).map_err(|_| corrupt())?;
            let value = json::parse(
                source,
                json::Limits {
                    bytes: EVENT_LIMIT,
                    nodes: 500_000,
                    depth: 64,
                },
            )
            .map_err(|_| corrupt())?;
            event(usize::try_from(turn).map_err(|_| corrupt())?, value)?;
        }
        position += HEADER as u64 + length as u64;
    }
    if rolling != expected_hash {
        return Err(corrupt());
    }
    Ok(())
}

pub(super) fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "committed session log is corrupt",
    )
}
