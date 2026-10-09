//! Session-owned attachment storage. Each asset keeps the exact imported bytes
//! under `<id>/<name>` and its metadata in `<id>.json`; an optional
//! `<id>.view.png` or `.jpg` holds a provider-ready image derived from it.

use super::{Attachment, Kind, media};
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Largest file an attachment may hold.
pub const LIMIT: u64 = 1024 * 1024 * 1024;
/// Images above these bounds are sent as a derived, smaller copy.
const VIEW_BYTES: u64 = 3_750_000;
const VIEW_PIXELS: u32 = 8000;
/// Unreferenced assets younger than this may belong to another process's
/// draft that has not reached disk yet.
const GRACE: Duration = Duration::from_secs(600);
const HEAD: usize = 64 * 1024;
const PDF_SCAN: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Pool {
    directory: PathBuf,
}

pub struct Stored {
    pub attachment: Attachment,
    pub path: PathBuf,
    pub source: Option<String>,
}

impl Pool {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    #[cfg(test)]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Copies a regular file into the pool. The copy is independent of the
    /// source, which may change or disappear afterwards.
    pub fn import_file(
        &self,
        source: &Path,
        cancellation: &Cancellation,
    ) -> Result<Attachment, String> {
        let shown = source.display();
        let metadata =
            fs::metadata(source).map_err(|error| format!("Cannot attach {shown}: {error}"))?;
        if !metadata.is_file() {
            return Err(format!("Cannot attach {shown}: not a regular file"));
        }
        if metadata.len() > LIMIT {
            return Err(format!(
                "Cannot attach {shown}: larger than {}",
                super::size(LIMIT)
            ));
        }
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".into());
        let mut input =
            File::open(source).map_err(|error| format!("Cannot attach {shown}: {error}"))?;
        let canonical = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
        self.store(
            &name,
            &mut input,
            Some(crate::scratch::environment_path(&canonical)),
            cancellation,
        )
    }

    pub fn import_bytes(&self, name: &str, bytes: &[u8]) -> Result<Attachment, String> {
        self.store(name, &mut &bytes[..], None, &Cancellation::default())
    }

    fn store(
        &self,
        name: &str,
        input: &mut dyn Read,
        source: Option<PathBuf>,
        cancellation: &Cancellation,
    ) -> Result<Attachment, String> {
        let id = super::new_id();
        let folder = self.directory.join(&id);
        let file = disk_name(name);
        let result = (|| {
            fs::create_dir_all(&folder)
                .map_err(|error| format!("Cannot create attachment storage: {error}"))?;
            let path = folder.join(&file);
            let size = copy(input, &path, cancellation)?;
            let attachment = describe(&id, name, &path, size)?;
            if attachment.kind() == Kind::Image && !direct(&attachment) {
                let data = fs::read(&path).map_err(|error| error.to_string())?;
                // Without a converter the original stays attached; the model
                // receives a note instead of an image part.
                if let Ok((extension, bytes)) = super::capture::convert(&data, cancellation) {
                    write_new(&self.view_path(&id, extension), &bytes)?;
                }
            }
            let mut record = attachment.value();
            if let Value::Object(fields) = &mut record {
                fields.insert("format".into(), Value::string("jecode.attachment"));
                fields.insert("version".into(), Value::number(1));
                fields.insert("file".into(), Value::string(&file));
                fields.insert("imported".into(), Value::number(crate::sessions::now()));
                if let Some(source) = &source {
                    fields.insert("source".into(), Value::string(source.to_string_lossy()));
                }
            }
            write_new(
                &self.directory.join(format!("{id}.json")),
                record.pretty().as_bytes(),
            )?;
            Ok(attachment)
        })();
        if result.is_err() {
            self.remove(&id);
        }
        result
    }

    pub fn load(&self, id: &str) -> Result<Stored, String> {
        if !super::valid_id(id) {
            return Err(format!("Unknown attachment: {id}"));
        }
        let text = fs::read_to_string(self.directory.join(format!("{id}.json")))
            .map_err(|_| format!("Attachment {id} is no longer stored"))?;
        let record = json::parse(&text)?;
        let file = record
            .get("file")
            .and_then(Value::as_str)
            .filter(|file| *file == disk_name(file))
            .ok_or("Attachment metadata is damaged")?;
        Ok(Stored {
            attachment: Attachment::parse(&record)?,
            path: self.directory.join(id).join(file),
            source: record.get("source").and_then(Value::as_str).map(Into::into),
        })
    }

    /// The image bytes to send to a provider, if the pool has a usable form.
    pub fn view(&self, attachment: &Attachment) -> Result<Option<(String, Vec<u8>)>, String> {
        let stored = self.load(&attachment.id)?;
        if direct(&stored.attachment) {
            let bytes = fs::read(&stored.path)
                .map_err(|error| format!("Cannot read attachment {}: {error}", attachment.id))?;
            return Ok(Some((stored.attachment.media, bytes)));
        }
        for (extension, media) in [("png", "image/png"), ("jpg", "image/jpeg")] {
            if let Ok(bytes) = fs::read(self.view_path(&attachment.id, extension)) {
                return Ok(Some((media.into(), bytes)));
            }
        }
        Ok(None)
    }

    fn view_path(&self, id: &str, extension: &str) -> PathBuf {
        self.directory.join(format!("{id}.view.{extension}"))
    }

    pub fn copy_to(&self, attachment: &Attachment, target: &Path) -> Result<PathBuf, String> {
        let stored = self.load(&attachment.id)?;
        let folder = target.join(&attachment.id);
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let destination = folder.join(stored.path.file_name().unwrap_or_default());
        fs::copy(&stored.path, &destination)
            .map_err(|error| format!("Could not copy attachment {}: {error}", attachment.id))?;
        Ok(destination)
    }

    fn remove(&self, id: &str) {
        let _ = fs::remove_dir_all(self.directory.join(id));
        for suffix in ["json", "partial", "view.png", "view.jpg", "view.partial"] {
            let _ = fs::remove_file(self.directory.join(format!("{id}.{suffix}")));
        }
    }

    /// Removes assets no session file, live state or recent import refers to.
    /// `released` assets skip the grace period: their owner was deleted.
    pub fn collect(
        &self,
        sessions: &Path,
        live: &BTreeSet<String>,
        released: &BTreeSet<String>,
    ) -> Result<usize, String> {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return Ok(0);
        };
        let mut referenced = live.clone();
        for entry in fs::read_dir(sessions)
            .map_err(|error| format!("Cannot inspect sessions: {error}"))?
            .flatten()
        {
            // Session locks hold no references and stay locked while open.
            let lock = entry.file_name().to_string_lossy().ends_with(".lock");
            if !lock && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                let bytes = fs::read(entry.path())
                    .map_err(|error| format!("Cannot inspect sessions: {error}"))?;
                referenced.extend(references(&bytes));
            }
        }
        let now = SystemTime::now();
        // The youngest entry of an asset decides its age.
        let mut ages = std::collections::BTreeMap::<String, Duration>::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name.split('.').next().unwrap_or_default().to_owned();
            if !super::valid_id(&id) {
                continue;
            }
            let age = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .unwrap_or_default();
            ages.entry(id)
                .and_modify(|current| *current = (*current).min(age))
                .or_insert(age);
        }
        let mut removed = 0;
        for (id, age) in ages {
            if !referenced.contains(&id) && (released.contains(&id) || age >= GRACE) {
                self.remove(&id);
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// Attachment ids mentioned in serialized data.
pub fn references(bytes: &[u8]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut at = 0;
    while let Some(offset) = bytes[at..].windows(4).position(|window| window == b"att-") {
        let start = at + offset;
        let end = start
            + 4
            + bytes[start + 4..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit() || **byte == b'-')
                .count();
        let id = String::from_utf8_lossy(&bytes[start..end]).into_owned();
        if super::valid_id(&id) {
            found.insert(id);
        }
        at = end;
    }
    found
}

fn direct(attachment: &Attachment) -> bool {
    media::PROVIDER_IMAGES.contains(&attachment.media.as_str())
        && attachment.size <= VIEW_BYTES
        && attachment.width.unwrap_or(0) <= VIEW_PIXELS
        && attachment.height.unwrap_or(0) <= VIEW_PIXELS
}

fn describe(id: &str, name: &str, path: &Path, size: u64) -> Result<Attachment, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut head = Vec::with_capacity(HEAD);
    (&mut file)
        .take(HEAD as u64)
        .read_to_end(&mut head)
        .map_err(|error| error.to_string())?;
    let detected = media::detect(&head, name, size <= HEAD as u64);
    let pages = if detected.media == "application/pdf" {
        let mut data = Vec::new();
        File::open(path)
            .and_then(|file| file.take(PDF_SCAN).read_to_end(&mut data))
            .map_err(|error| error.to_string())?;
        media::pdf_pages(&data)
    } else {
        None
    };
    Ok(Attachment {
        id: id.into(),
        name: name.into(),
        media: detected.media,
        size,
        width: detected.width,
        height: detected.height,
        pages,
    })
}

fn copy(input: &mut dyn Read, path: &Path, cancellation: &Cancellation) -> Result<u64, String> {
    let mut output = File::create_new(path)
        .map_err(|error| format!("Cannot create attachment copy: {error}"))?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut size = 0u64;
    loop {
        if cancellation.requested() {
            return Err("Attachment import cancelled".into());
        }
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("Cannot read attachment source: {error}"))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        if size > LIMIT {
            return Err(format!("Attachment is larger than {}", super::size(LIMIT)));
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("Cannot write attachment copy: {error}"))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("Cannot write attachment copy: {error}"))?;
    Ok(size)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let partial = path.with_extension("partial");
    let result = File::create(&partial)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|_| fs::rename(&partial, path));
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(format!("Cannot write attachment metadata: {error}"));
    }
    Ok(())
}

/// A file name valid on Windows and Unix. The original name stays in metadata.
pub fn disk_name(name: &str) -> String {
    let mut clean = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .trim_start()
        .to_owned();
    let stem = clean
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        clean.insert(0, '_');
    }
    if clean.is_empty() || clean == "." || clean == ".." {
        clean = "attachment".into();
    }
    if clean.len() > 120 {
        let extension = clean
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .filter(|extension| extension.len() <= 16)
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default();
        let mut end = 120 - extension.len();
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        clean = format!("{}{extension}", &clean[..end]);
    }
    clean
}
