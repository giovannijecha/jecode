//! Segmented append-only journal of one archive generation. Appends and
//! replay run under the archive's store lock; object reads use positional
//! reads of immutable frames and need no lock.
use super::files;
use super::frame::{self, HEADER, Kind, ObjectMeta};
use super::layout;
use super::locks::Shared;
use super::pressure::{self, Pressure};
use super::quota;
use super::record::Record;
use crate::cancel::Cancellation;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const SEGMENT_LIMIT: u64 = 64 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub segment: u64,
    /// Offset of the frame header.
    pub offset: u64,
    pub length: u64,
    pub checksum: u64,
}

pub enum Entry {
    Record(Record),
    Object(ObjectMeta, Location, Vec<u64>),
}

pub struct Journal {
    archive: PathBuf,
    pressure: Arc<Pressure>,
    pub min_free_bytes: u64,
    pub generation: u64,
    _generation: Shared,
    segment: u64,
    offset: u64,
    /// Lengths of segments already passed while following this generation.
    sealed_bytes: u64,
    writer: Option<File>,
    /// Stays open on the followed segment: on Windows, reopening a file just
    /// appended to makes real-time antivirus rescan it, about 9 ms each time.
    follower: Option<(u64, File)>,
    readers: Mutex<HashMap<u64, File>>,
    /// Complete frames that failed validation and were skipped.
    pub damage: Vec<String>,
    /// Damage that ends the readable part of the current segment; new frames
    /// go to a fresh segment so readers reach them after skipping it.
    pub blocked: Option<String>,
    /// Makes appends fail, to prove that tools proceed without protection.
    #[cfg(test)]
    pub fault: Option<String>,
    #[cfg(test)]
    pub sync_fault: Option<String>,
    pub quota_limit: u64,
}

pub fn generation_directory(archive: &Path, generation: u64) -> PathBuf {
    layout::generation_directory(archive, generation)
}

pub fn generation_lock(archive: &Path, generation: u64) -> PathBuf {
    layout::generation_lock(archive, generation)
}

pub fn current_generation(archive: &Path) -> Result<u64, String> {
    match fs::read_to_string(layout::current(archive)) {
        Ok(text) => text
            .trim()
            .parse()
            .map_err(|_| "The checkpoint archive CURRENT file is damaged".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(1),
        Err(error) => Err(format!("Could not read checkpoint archive: {error}")),
    }
}

/// Whether everything after the reader position is zero, as a file system
/// leaves an extended but unwritten tail after a crash.
fn zeros(reader: &mut impl Read) -> Result<bool, String> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) if buffer[..count].iter().all(|byte| *byte == 0) => {}
            Ok(_) => return Ok(false),
            Err(error) => return Err(format!("Could not read checkpoint journal: {error}")),
        }
    }
}

impl Journal {
    /// Opens the current generation. Callers hold the store lock.
    pub fn open(archive: &Path) -> Result<Self, String> {
        let generation = current_generation(archive)?;
        if layout::current(archive).exists() {
            Self::open_existing_generation(archive, generation)
        } else {
            Self::open_generation(archive, generation)
        }
    }

    /// Opens one generation for reading from its start, or for writing a new one.
    pub fn open_generation(archive: &Path, generation: u64) -> Result<Self, String> {
        Self::open_with_creation(archive, generation, true)
    }

    /// Follows a published generation without inventing one if it went missing.
    pub fn open_existing_generation(archive: &Path, generation: u64) -> Result<Self, String> {
        Self::open_with_creation(archive, generation, false)
    }

    fn open_with_creation(archive: &Path, generation: u64, create: bool) -> Result<Self, String> {
        let lock = Shared::acquire(&generation_lock(archive, generation))
            .map_err(|error| format!("Could not lock checkpoint archive: {error}"))?;
        let directory = generation_directory(archive, generation);
        if create {
            let created = !directory.exists();
            files::private_directory(&directory)
                .map_err(|error| format!("Could not create checkpoint archive: {error}"))?;
            if created {
                files::sync_directory(&layout::history(archive))
                    .map_err(|error| format!("Could not flush checkpoint archive: {error}"))?;
            }
        } else if !directory.join("000001.log").is_file() {
            return Err(format!(
                "Checkpoint archive generation {generation} is missing; the previous history was preserved"
            ));
        }
        Ok(Self {
            archive: archive.to_path_buf(),
            pressure: pressure::for_archive(archive)?,
            min_free_bytes: pressure::DEFAULT_MIN_FREE_BYTES,
            generation,
            _generation: lock,
            segment: 1,
            offset: 0,
            sealed_bytes: 0,
            writer: None,
            follower: None,
            readers: Mutex::new(HashMap::new()),
            damage: Vec::new(),
            blocked: None,
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            sync_fault: None,
            quota_limit: u64::MAX,
        })
    }

    pub fn segment_path(&self, segment: u64) -> PathBuf {
        generation_directory(&self.archive, self.generation).join(format!("{segment:06}.log"))
    }

    /// Replays frames appended since the last call, by any process.
    pub fn catch_up(&mut self, apply: &mut dyn FnMut(Entry)) -> Result<(), String> {
        self.catch_up_cancellable(apply, &crate::cancel::Cancellation::default())
    }

    pub fn catch_up_cancellable(
        &mut self,
        apply: &mut dyn FnMut(Entry),
        cancellation: &crate::cancel::Cancellation,
    ) -> Result<(), String> {
        loop {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let file = match self.follower.take() {
                Some((segment, file)) if segment == self.segment => file,
                _ => match File::open(self.segment_path(self.segment)) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                    Err(error) => {
                        return Err(format!("Could not read checkpoint journal: {error}"));
                    }
                },
            };
            let length = file
                .metadata()
                .map_err(|error| format!("Could not read checkpoint journal: {error}"))?
                .len();
            if self.offset < length {
                self.replay(&file, length, apply, cancellation)?;
            }
            if self.offset >= length && self.segment_path(self.segment + 1).exists() {
                self.sealed_bytes = self.sealed_bytes.saturating_add(length);
                self.segment += 1;
                self.offset = 0;
                self.writer = None;
                self.blocked = None;
                continue;
            }
            self.follower = Some((self.segment, file));
            return Ok(());
        }
    }

    fn replay(
        &mut self,
        file: &File,
        length: u64,
        apply: &mut dyn FnMut(Entry),
        cancellation: &crate::cancel::Cancellation,
    ) -> Result<(), String> {
        let io = |error: std::io::Error| format!("Could not read checkpoint journal: {error}");
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        reader.seek(SeekFrom::Start(self.offset)).map_err(io)?;
        while self.offset < length {
            if cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let remaining = length - self.offset;
            let mut bytes = [0u8; HEADER];
            if remaining < HEADER as u64 {
                return self.tail(length, "a partial frame header");
            }
            reader.read_exact(&mut bytes).map_err(io)?;
            let header = match frame::header(&bytes) {
                Ok(header) => header,
                Err(_) if bytes.iter().all(|byte| *byte == 0) && zeros(&mut reader)? => {
                    return self.tail(length, "an unwritten frame");
                }
                Err(error) => {
                    let message = format!(
                        "segment {} offset {}: {error}; later history in this segment is unreadable",
                        self.segment, self.offset
                    );
                    self.blocked = Some(message.clone());
                    self.damage.push(message);
                    // Later segments are independent and still replay.
                    self.offset = length;
                    return Ok(());
                }
            };
            let end = self
                .offset
                .checked_add(HEADER as u64)
                .and_then(|end| end.checked_add(header.length));
            let Some(end) = end.filter(|end| *end <= length) else {
                // A damaged length is indistinguishable from an interrupted
                // payload. Keep all bytes, including any later valid frames.
                let message = format!(
                    "segment {} offset {}: frame length exceeds the file; later history in this segment is unreadable",
                    self.segment, self.offset
                );
                self.blocked = Some(message.clone());
                self.damage.push(message);
                self.offset = length;
                return Ok(());
            };
            let location = Location {
                segment: self.segment,
                offset: self.offset,
                length: header.length,
                checksum: header.checksum,
            };
            match Kind::from(header.kind) {
                Some(Kind::Object) if header.length >= ObjectMeta::SIZE as u64 => {
                    let mut meta = [0u8; ObjectMeta::SIZE];
                    reader.read_exact(&mut meta).map_err(io)?;
                    reader
                        .seek_relative((header.length - ObjectMeta::SIZE as u64) as i64)
                        .map_err(io)?;
                    match ObjectMeta::decode(&meta) {
                        Ok(object) => {
                            let chunks = if let frame::Encoding::Chunks { count } = object.encoding
                            {
                                let bytes = header.length - ObjectMeta::SIZE as u64;
                                if bytes > frame::MANIFEST_LIMIT
                                    || count.checked_mul(8) != Some(bytes)
                                {
                                    self.skip(location, "invalid chunk manifest length");
                                    self.offset = end;
                                    continue;
                                }
                                reader.seek_relative(-(bytes as i64)).map_err(io)?;
                                let mut payload = Vec::with_capacity(header.length as usize);
                                payload.extend_from_slice(&meta);
                                payload.resize(header.length as usize, 0);
                                reader
                                    .read_exact(&mut payload[ObjectMeta::SIZE..])
                                    .map_err(io)?;
                                if frame::checksum(header.kind, &payload) != header.checksum {
                                    self.skip(location, "chunk manifest checksum mismatch");
                                    self.offset = end;
                                    continue;
                                }
                                payload[ObjectMeta::SIZE..]
                                    .chunks_exact(8)
                                    .map(|id| u64::from_le_bytes(id.try_into().unwrap_or_default()))
                                    .collect()
                            } else {
                                // Full and delta payload checksums are verified on reads.
                                Vec::new()
                            };
                            apply(Entry::Object(object, location, chunks));
                        }
                        Err(error) => self.skip(location, &error),
                    }
                }
                Some(kind) if kind != Kind::Object && header.length <= frame::METADATA_LIMIT => {
                    let mut payload = vec![0u8; header.length as usize];
                    reader.read_exact(&mut payload).map_err(io)?;
                    if frame::checksum(header.kind, &payload) != header.checksum {
                        self.skip(location, "checksum mismatch");
                    } else {
                        match Record::decode(kind, &payload) {
                            Ok(record) => apply(Entry::Record(record)),
                            Err(error) => self.skip(location, &error),
                        }
                    }
                }
                _ => {
                    reader.seek_relative(header.length as i64).map_err(io)?;
                    self.skip(location, "unknown or invalid frame");
                }
            }
            self.offset = end;
        }
        Ok(())
    }

    fn skip(&mut self, location: Location, reason: &str) {
        self.damage.push(format!(
            "segment {} offset {}: {reason}; the frame was skipped",
            location.segment, location.offset
        ));
    }

    /// Truncates a provably incomplete header or unwritten zero tail, only in
    /// the newest segment. A complete header with a suspect length is kept.
    fn tail(&mut self, length: u64, what: &str) -> Result<(), String> {
        if self.segment_path(self.segment + 1).exists() {
            let message = format!(
                "segment {} ends with {what} although a newer segment exists",
                self.segment
            );
            self.blocked = Some(message.clone());
            self.damage.push(message);
            self.offset = length;
            return Ok(());
        }
        let file = OpenOptions::new()
            .write(true)
            .open(self.segment_path(self.segment))
            .and_then(|file| {
                file.set_len(self.offset)?;
                file.sync_all()
            });
        if let Err(error) = file {
            return Err(format!(
                "Could not repair the incomplete checkpoint journal tail: {error}"
            ));
        }
        self.damage.push(format!(
            "segment {} offset {}: removed {} bytes of {what} left by an interrupted write",
            self.segment,
            self.offset,
            length - self.offset
        ));
        self.writer = None;
        Ok(())
    }

    /// Appends frames after `catch_up`. Returns the segment and offset of the
    /// first byte. Durable appends reach stable storage before returning.
    pub fn append(&mut self, bytes: &[u8], durable: bool) -> Result<(u64, u64), String> {
        self.append_checked(bytes, durable, false, false, None, None)
    }

    /// `used` is measured under the store lock and updated after each
    /// successful append in the same transaction.
    pub fn append_counted(
        &mut self,
        bytes: &[u8],
        durable: bool,
        reserved: bool,
        metadata_only: bool,
        used: quota::Usage,
        cancellation: Option<&Cancellation>,
    ) -> Result<(u64, u64), String> {
        self.append_checked(
            bytes,
            durable,
            reserved,
            metadata_only,
            Some(used),
            cancellation,
        )
    }

    /// Verifies space for the retained generation before compaction creates it.
    /// The append guard still checks each batch as concurrent writes may extend
    /// the old generation after this estimate.
    pub fn ensure_scratch(&self, projected: u64) -> Result<(), String> {
        self.pressure
            .ensure_scratch(&self.archive, projected, self.min_free_bytes)
    }

    fn append_checked(
        &mut self,
        bytes: &[u8],
        durable: bool,
        reserved: bool,
        metadata_only: bool,
        used: Option<quota::Usage>,
        cancellation: Option<&Cancellation>,
    ) -> Result<(u64, u64), String> {
        #[cfg(test)]
        if let Some(fault) = &self.fault {
            return Err(fault.clone());
        }
        let limit = self.quota_limit;
        if limit != u64::MAX {
            match used {
                Some(used) => quota::admit_generation(
                    used,
                    bytes.len() as u64,
                    metadata_only,
                    reserved,
                    limit,
                )?,
                None => quota::admit(&self.archive, bytes.len() as u64, reserved, limit)?,
            }
        }
        frame::validate_lengths(bytes)?;
        self.pressure.claim(
            &self.archive,
            bytes.len() as u64,
            if reserved && metadata_only && bytes.len() as u64 <= quota::RESERVE {
                0
            } else {
                self.min_free_bytes
            },
            cancellation,
        )?;
        let io = |error: std::io::Error| format!("Could not write checkpoint journal: {error}");
        // The damaged bytes stay for inspection; appending after them would
        // hide the new frames from every reader.
        if self.offset >= SEGMENT_LIMIT || self.blocked.is_some() {
            self.sync()?;
            self.sealed_bytes = self.sealed_bytes.saturating_add(self.offset);
            self.segment += 1;
            self.offset = 0;
            self.writer = None;
            self.blocked = None;
        }
        if self.writer.is_none() {
            let path = self.segment_path(self.segment);
            let created = !path.exists();
            let file = files::options()
                .append(true)
                .create(true)
                .open(&path)
                .map_err(io)?;
            if created {
                file.sync_all().map_err(io)?;
                files::sync_directory(path.parent().unwrap_or(&self.archive)).map_err(io)?;
            }
            self.writer = Some(file);
        }
        let writer = self.writer.as_mut().ok_or("journal writer unavailable")?;
        if writer.metadata().map_err(io)?.len() != self.offset {
            self.writer = None;
            return Err("The checkpoint journal changed outside the store lock".into());
        }
        let start = self.offset;
        writer.write_all(bytes).map_err(io)?;
        if durable {
            writer.sync_data().map_err(io)?;
        }
        self.offset += bytes.len() as u64;
        Ok((self.segment, start))
    }

    pub fn sync(&mut self) -> Result<(), String> {
        #[cfg(test)]
        if let Some(fault) = &self.sync_fault {
            return Err(fault.clone());
        }
        if let Some(writer) = &self.writer {
            writer
                .sync_data()
                .map_err(|error| format!("Could not flush checkpoint journal: {error}"))?;
        } else if self.segment_path(self.segment).exists() {
            // Another process may have appended the frames this process read.
            // A reused point still needs to persist those frames before work.
            files::options()
                .read(true)
                .write(true)
                .open(self.segment_path(self.segment))
                .and_then(|file| file.sync_data())
                .map_err(|error| format!("Could not flush checkpoint journal: {error}"))?;
        }
        Ok(())
    }

    /// Reads and verifies one frame payload.
    pub fn read(&self, location: &Location) -> Result<Vec<u8>, String> {
        let mut readers = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        let file = match readers.entry(location.segment) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                File::open(self.segment_path(location.segment))
                    .map_err(|error| format!("Could not open checkpoint journal: {error}"))?,
            ),
        };
        let mut payload = vec![0u8; location.length as usize];
        file.seek(SeekFrom::Start(location.offset + HEADER as u64))
            .and_then(|_| file.read_exact(&mut payload))
            .map_err(|error| format!("Could not read checkpoint object: {error}"))?;
        if frame::checksum(Kind::Object as u8, &payload) != location.checksum {
            return Err(format!(
                "Checkpoint object at segment {} offset {} is damaged (checksum mismatch)",
                location.segment, location.offset
            ));
        }
        Ok(payload)
    }

    pub fn position(&self) -> (u64, u64) {
        (self.segment, self.offset)
    }

    /// Logical bytes of this generation's journal segments, including records
    /// and damaged bytes that remain for inspection.
    pub fn stored_bytes(&self) -> u64 {
        self.sealed_bytes.saturating_add(self.offset)
    }

    /// Continues reading at a position another reader of this generation reached.
    pub fn seek(&mut self, (segment, offset): (u64, u64)) {
        self.segment = segment;
        self.offset = offset;
        self.writer = None;
    }
}
