//! Private, durable file versions. The workspace rename never crosses into this store.
use super::{Budget, platform};
use crate::{
    json::{self, Value},
    state::Store,
    tls::crypto::sha256::Sha256,
};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MANIFEST_LIMIT: usize = 512 * 1024;
pub(crate) struct Origin<'a> {
    pub workspace: &'a Path,
    pub target: &'a Path,
    pub session: Option<&'a str>,
    pub operation: &'a str,
    pub adjacent: &'a Path,
}
#[derive(Debug)]
pub(crate) struct CaptureError {
    pub id: Option<String>,
    pub source: io::Error,
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}
impl From<io::Error> for CaptureError {
    fn from(source: io::Error) -> Self {
        Self { id: None, source }
    }
}
impl From<io::ErrorKind> for CaptureError {
    fn from(kind: io::ErrorKind) -> Self {
        Self::from(io::Error::from(kind))
    }
}

#[derive(Clone)]
pub struct RecoveryStore {
    files: Store,
    #[cfg(test)]
    fail_record_in: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    fail_capture: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Debug)]
pub struct Version {
    pub id: String,
    pub workspace: String,
    pub target: String,
    pub session: Option<String>,
    pub operation: String,
    pub state: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub adjacent: String,
    pub(crate) schema: u64,
    pub(crate) before_sha256: Option<[u8; 32]>,
    pub(crate) after_sha256: Option<[u8; 32]>,
    pub(crate) policy: Vec<u8>,
    pub(crate) after_policy: Vec<u8>,
    pub(crate) before_modified: SystemTime,
    pub(crate) after_modified: SystemTime,
    pub(crate) original_identity: (u64, u64),
    pub(crate) staged_identity: (u64, u64),
    pub(crate) restore_identity: Option<(u64, u64)>,
    pub(crate) restore_stage: Option<String>,
    pub(crate) published_identity: Option<(u64, u64)>,
    pub(crate) replaced_identity: Option<(u64, u64)>,
}

impl RecoveryStore {
    pub fn user() -> io::Result<Self> {
        Self::in_store(&Store::user()?)
    }
    pub fn in_store(store: &Store) -> io::Result<Self> {
        Ok(Self {
            files: store.directory("recoveries")?,
            #[cfg(test)]
            fail_record_in: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            #[cfg(test)]
            fail_capture: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    pub fn root(&self) -> &Path {
        self.files.root()
    }
    pub fn list(&self) -> io::Result<Vec<Version>> {
        self.files
            .names()?
            .into_iter()
            .filter_map(|name| name.strip_suffix(".json").map(str::to_owned))
            .map(|id| self.get(&id))
            .collect()
    }
    pub fn get(&self, id: &str) -> io::Result<Version> {
        valid_id(id)?;
        let text = self
            .files
            .read(&format!("{id}.json"), MANIFEST_LIMIT)?
            .ok_or(io::ErrorKind::NotFound)?;
        let value = json::parse(
            &text,
            json::Limits {
                bytes: MANIFEST_LIMIT,
                ..Default::default()
            },
        )
        .map_err(|_| io::ErrorKind::InvalidData)?;
        let field = |key| {
            value
                .get(key)
                .and_then(Value::text)
                .map(str::to_owned)
                .ok_or(io::ErrorKind::InvalidData)
        };
        let number = |key| {
            value
                .get(key)
                .and_then(Value::unsigned)
                .ok_or(io::ErrorKind::InvalidData)
        };
        let schema = number("version")?;
        if !matches!(schema, 1 | 2) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let identity = |key| -> io::Result<Option<(u64, u64)>> {
            match value.get(key) {
                Some(Value::Array(parts)) if parts.len() == 2 => Ok(Some((
                    parts[0].unsigned().ok_or(io::ErrorKind::InvalidData)?,
                    parts[1].unsigned().ok_or(io::ErrorKind::InvalidData)?,
                ))),
                Some(Value::Null) => Ok(None),
                _ => Err(io::ErrorKind::InvalidData.into()),
            }
        };
        if field("id")? != id {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let digest = |key| -> io::Result<Option<[u8; 32]>> {
            if schema == 1 {
                return Ok(None);
            }
            match value.get(key) {
                Some(Value::String(text)) => decode_hex(text)?
                    .try_into()
                    .map(Some)
                    .map_err(|_| io::ErrorKind::InvalidData.into()),
                Some(Value::Null) => Ok(None),
                _ => Err(io::ErrorKind::InvalidData.into()),
            }
        };
        let before_sha256 = digest("before_sha256")?;
        let after_sha256 = digest("after_sha256")?;
        let state = field("state")?;
        if schema == 2
            && state != "capturing"
            && (before_sha256.is_none() || after_sha256.is_none())
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(Version {
            id: id.into(),
            workspace: field("workspace")?,
            target: field("target")?,
            session: value
                .get("session")
                .and_then(Value::text)
                .map(str::to_owned),
            operation: field("operation")?,
            state,
            before_bytes: number("before_bytes")?,
            after_bytes: number("after_bytes")?,
            adjacent: field("adjacent")?,
            schema,
            before_sha256,
            after_sha256,
            policy: decode_hex(&field("policy")?)?,
            after_policy: decode_hex(&field("after_policy")?)?,
            before_modified: decode_time(&field("before_modified")?)?,
            after_modified: decode_time(&field("after_modified")?)?,
            original_identity: identity("original_identity")?.ok_or(io::ErrorKind::InvalidData)?,
            staged_identity: identity("staged_identity")?.ok_or(io::ErrorKind::InvalidData)?,
            restore_identity: identity("restore_identity")?,
            restore_stage: if schema == 2 {
                match value.get("restore_stage") {
                    Some(Value::String(name)) => Some(name.to_owned()),
                    Some(Value::Null) => None,
                    _ => return Err(io::ErrorKind::InvalidData.into()),
                }
            } else {
                None
            },
            published_identity: identity("published_identity")?,
            replaced_identity: if schema == 2 {
                identity("replaced_identity")?
            } else {
                None
            },
        })
    }
    pub fn original(&self, id: &str) -> io::Result<File> {
        if self.get(id)?.state == "capturing" {
            return Err(io::Error::other("original capture is incomplete"));
        }
        self.file(id, "before")
    }
    pub(crate) fn capture(
        &self,
        origin: Origin<'_>,
        original: &mut File,
        staged: &mut File,
        budget: &Budget<'_>,
    ) -> Result<Version, CaptureError> {
        let workspace = origin
            .workspace
            .to_str()
            .ok_or(io::ErrorKind::InvalidInput)?
            .to_owned();
        let target = origin
            .target
            .to_str()
            .ok_or(io::ErrorKind::InvalidInput)?
            .to_owned();
        let adjacent = origin
            .adjacent
            .to_str()
            .ok_or(io::ErrorKind::InvalidInput)?
            .to_owned();
        let policy = platform::capture_policy(original)?;
        let after_policy = platform::capture_policy(staged)?;
        let before_modified = original.metadata()?.modified()?;
        let after_modified = staged.metadata()?.modified()?;
        let id = unique_id();
        let mut version = Version {
            id: id.clone(),
            workspace,
            target,
            session: origin.session.map(str::to_owned),
            operation: origin.operation.into(),
            state: "capturing".into(),
            before_bytes: original.metadata()?.len(),
            after_bytes: staged.metadata()?.len(),
            adjacent,
            schema: 2,
            before_sha256: None,
            after_sha256: None,
            policy,
            after_policy,
            before_modified,
            after_modified,
            original_identity: platform::identity(original)?,
            staged_identity: platform::identity(staged)?,
            restore_identity: None,
            restore_stage: None,
            published_identity: None,
            replaced_identity: None,
        };
        // The association is durable before any potentially partial private copy.
        // A capturing record never authorizes a workspace publication.
        self.write_new(&version)?;
        #[cfg(test)]
        if self.fail_capture.swap(false, Ordering::AcqRel) {
            return Err(CaptureError {
                id: Some(id),
                source: io::Error::other("injected failure after capture manifest"),
            });
        }
        (|| -> io::Result<Version> {
            let mut before = self.files.data_file(&format!("{id}.before"), false)?;
            let mut after = self.files.data_file(&format!("{id}.after"), false)?;
            let (before_bytes, before_sha256) = copy(original, &mut before, budget)?;
            let (after_bytes, after_sha256) = copy(staged, &mut after, budget)?;
            if before_bytes != version.before_bytes || after_bytes != version.after_bytes {
                return Err(io::Error::other(
                    "source changed while private recovery was captured",
                ));
            }
            before.sync_all()?;
            after.sync_all()?;
            self.files.sync_root()?;
            if !same(original, &mut before, budget)? || !same(staged, &mut after, budget)? {
                return Err(io::Error::other(
                    "private recovery copy differs from the prepared file",
                ));
            }
            version.before_sha256 = Some(before_sha256);
            version.after_sha256 = Some(after_sha256);
            version.state = "captured".into();
            self.write(&version)?;
            Ok(version)
        })()
        .map_err(|source| CaptureError {
            id: Some(id),
            source,
        })
    }
    pub(crate) fn record(
        &self,
        version: &mut Version,
        state: &str,
        identity: Option<(u64, u64)>,
    ) -> io::Result<()> {
        #[cfg(test)]
        if self
            .fail_record_in
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .ok()
            == Some(1)
        {
            return Err(io::Error::other("injected recovery checkpoint failure"));
        }
        let mut updated = version.clone();
        updated.state = state.into();
        updated.published_identity = identity;
        self.write(&updated)?;
        *version = updated;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn fail_next_record(&self) {
        self.fail_record_in.store(1, Ordering::Release);
    }
    #[cfg(test)]
    pub(crate) fn fail_record_after(&self, successful_records: usize) {
        self.fail_record_in
            .store(successful_records + 1, Ordering::Release);
    }
    #[cfg(test)]
    pub(crate) fn fail_next_capture(&self) {
        self.fail_capture.store(true, Ordering::Release);
    }
    fn write(&self, version: &Version) -> io::Result<()> {
        self.files
            .replace(&format!("{}.json", version.id), &encode_manifest(version)?)
    }
    fn write_new(&self, version: &Version) -> io::Result<()> {
        let mut file = self
            .files
            .data_file(&format!("{}.json", version.id), false)?;
        file.write_all(encode_manifest(version)?.as_bytes())?;
        file.sync_all()?;
        self.files.sync_root()
    }
    pub(crate) fn file(&self, id: &str, suffix: &str) -> io::Result<File> {
        valid_id(id)?;
        self.files.read_file(&format!("{id}.{suffix}"))
    }
    pub(crate) fn verified_file(
        &self,
        version: &Version,
        suffix: &str,
        budget: &Budget<'_>,
    ) -> io::Result<File> {
        let (length, expected) = version.expected(suffix)?;
        let mut file = self.file(&version.id, suffix)?;
        let (actual_length, actual) = digest(&mut file, budget)?;
        if actual_length != length || actual != expected {
            return Err(io::Error::other(format!(
                "retained {suffix} content failed capture-time integrity check"
            )));
        }
        file.rewind()?;
        Ok(file)
    }
    pub(crate) fn verify_pair(&self, version: &Version, budget: &Budget<'_>) -> io::Result<()> {
        self.verified_file(version, "before", budget)?;
        self.verified_file(version, "after", budget)?;
        Ok(())
    }
}
impl Version {
    pub(crate) fn expected(&self, suffix: &str) -> io::Result<(u64, [u8; 32])> {
        let value = match suffix {
            "before" => (self.before_bytes, self.before_sha256),
            "after" => (self.after_bytes, self.after_sha256),
            _ => return Err(io::ErrorKind::InvalidInput.into()),
        };
        value.1.map(|hash| (value.0, hash)).ok_or_else(|| {
            io::Error::other(
                "recovery predates capture-time integrity metadata; automatic restore and repair are unavailable",
            )
        })
    }
}
fn encode_manifest(version: &Version) -> io::Result<String> {
    let text = |value: &str| Value::String(value.into());
    let number = |value: u64| Value::Number(value.to_string());
    let identity = |value: Option<(u64, u64)>| {
        value.map_or(Value::Null, |(a, b)| {
            Value::Array(vec![number(a), number(b)])
        })
    };
    let value = json::object([
        ("version", number(version.schema)),
        ("id", text(&version.id)),
        ("workspace", text(&version.workspace)),
        ("target", text(&version.target)),
        (
            "session",
            version.session.as_deref().map_or(Value::Null, text),
        ),
        ("operation", text(&version.operation)),
        ("state", text(&version.state)),
        ("before_bytes", number(version.before_bytes)),
        ("after_bytes", number(version.after_bytes)),
        (
            "before_sha256",
            version
                .before_sha256
                .map_or(Value::Null, |hash| text(&encode_hex(&hash))),
        ),
        (
            "after_sha256",
            version
                .after_sha256
                .map_or(Value::Null, |hash| text(&encode_hex(&hash))),
        ),
        ("adjacent", text(&version.adjacent)),
        ("policy", text(&encode_hex(&version.policy))),
        ("after_policy", text(&encode_hex(&version.after_policy))),
        (
            "before_modified",
            text(&encode_time(version.before_modified)?),
        ),
        (
            "after_modified",
            text(&encode_time(version.after_modified)?),
        ),
        (
            "original_identity",
            identity(Some(version.original_identity)),
        ),
        ("staged_identity", identity(Some(version.staged_identity))),
        ("restore_identity", identity(version.restore_identity)),
        (
            "restore_stage",
            version.restore_stage.as_deref().map_or(Value::Null, text),
        ),
        ("published_identity", identity(version.published_identity)),
        ("replaced_identity", identity(version.replaced_identity)),
    ]);
    let encoded = json::encode(&value, MANIFEST_LIMIT).map_err(|_| io::ErrorKind::InvalidData)?;
    Ok(encoded)
}

fn valid_id(id: &str) -> io::Result<()> {
    if id.len() > 100
        || !id.starts_with("r-")
        || !id[2..].bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(())
}
fn unique_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "r-{time:x}-{:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn copy(source: &mut File, target: &mut File, budget: &Budget<'_>) -> io::Result<(u64, [u8; 32])> {
    source.rewind()?;
    let mut bytes = 0;
    let mut hash = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        budget.check().map_err(|_| io::ErrorKind::Interrupted)?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        target.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok((bytes, hash.finish()))
}
pub(crate) fn digest(source: &mut File, budget: &Budget<'_>) -> io::Result<(u64, [u8; 32])> {
    source.rewind()?;
    let mut bytes = 0;
    let mut hash = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        budget.check().map_err(|_| io::ErrorKind::Interrupted)?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok((bytes, hash.finish()))
}
pub(crate) fn same(
    source: &mut File,
    expected: &mut File,
    budget: &Budget<'_>,
) -> io::Result<bool> {
    source.rewind()?;
    expected.rewind()?;
    if source.metadata()?.len() != expected.metadata()?.len() {
        return Ok(false);
    }
    let mut left = [0; 16 * 1024];
    let mut right = [0; 16 * 1024];
    loop {
        budget.check().map_err(|_| io::ErrorKind::Interrupted)?;
        let count = source.read(&mut left)?;
        if count == 0 {
            return Ok(true);
        }
        expected.read_exact(&mut right[..count])?;
        if left[..count] != right[..count] {
            return Ok(false);
        }
    }
}
fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn decode_hex(text: &str) -> io::Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 131072 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| io::ErrorKind::InvalidData)?;
            u8::from_str_radix(text, 16).map_err(|_| io::ErrorKind::InvalidData.into())
        })
        .collect()
}
fn encode_time(time: SystemTime) -> io::Result<String> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| io::ErrorKind::InvalidData)?;
    Ok(format!(
        "{}:{}",
        duration.as_secs(),
        duration.subsec_nanos()
    ))
}
fn decode_time(text: &str) -> io::Result<SystemTime> {
    let (seconds, nanos) = text.split_once(':').ok_or(io::ErrorKind::InvalidData)?;
    let seconds: u64 = seconds.parse().map_err(|_| io::ErrorKind::InvalidData)?;
    let nanos: u32 = nanos.parse().map_err(|_| io::ErrorKind::InvalidData)?;
    if nanos >= 1_000_000_000 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(UNIX_EPOCH + std::time::Duration::new(seconds, nanos))
}
