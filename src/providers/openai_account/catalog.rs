//! Bounded account catalog metadata. Saved selections contain none of these fields.
use crate::{
    json::{self, Value},
    session::Model,
};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

pub const MAX_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const MAX_LEVELS: usize = 16;

/// Codex backend catalog route, distinct from the public API-key `/v1/models`.
/// The query is pinned to the official Codex stable release 0.156.1, published
/// 2026-09-23, and the route was checked at source revision 30fc6864cc1318121eca1843c217fe00ce1212f1.
/// Recheck it when updating the catalog contract; Jecode's package version is
/// not a Codex protocol compatibility claim.
pub const PATH: &str = "/backend-api/codex/models?client_version=0.156.1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Malformed,
    Empty,
    Limit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub default_effort: Option<String>,
    /// `None` means the service did not supply reliable capability metadata.
    pub efforts: Option<Vec<String>>,
    pub visible: bool,
    pub compatible: bool,
    /// Explicit service metadata only. Missing modalities are not image evidence.
    pub image: Support,
    priority: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    fetched_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Supported,
    Unknown,
    Unsupported,
}

impl Catalog {
    pub fn parse(body: &[u8]) -> Result<Self, Error> {
        if body.len() > MAX_BYTES {
            return Err(Error::Limit);
        }
        let text = std::str::from_utf8(body).map_err(|_| Error::Malformed)?;
        let root = json::parse(
            text,
            json::Limits {
                bytes: MAX_BYTES,
                nodes: 32768,
                depth: 24,
            },
        )
        .map_err(|_| Error::Malformed)?;
        let source = root
            .get("models")
            .and_then(Value::array)
            .ok_or(Error::Malformed)?;
        if source.len() > MAX_ENTRIES {
            return Err(Error::Limit);
        }
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        for item in source {
            let id = item
                .get("slug")
                .and_then(Value::text)
                .ok_or(Error::Malformed)?;
            if Model::new(id, None).is_none() || !seen.insert(id) {
                return Err(Error::Malformed);
            }
            let name = item
                .get("display_name")
                .and_then(Value::text)
                .filter(|name| !name.is_empty() && name.len() <= 128)
                .unwrap_or(id)
                .to_owned();
            let default_effort = match item.get("default_reasoning_level") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if Model::new(id, Some(value)).is_some() => {
                    Some(value.clone())
                }
                _ => return Err(Error::Malformed),
            };
            let efforts = match item.get("supported_reasoning_levels") {
                None => None,
                Some(Value::Array(levels)) if levels.len() <= MAX_LEVELS => {
                    let mut values = Vec::new();
                    for level in levels {
                        let effort = level
                            .get("effort")
                            .and_then(Value::text)
                            .ok_or(Error::Malformed)?;
                        if Model::new(id, Some(effort)).is_none()
                            || values.iter().any(|known| known == effort)
                        {
                            return Err(Error::Malformed);
                        }
                        values.push(effort.to_owned());
                    }
                    Some(values)
                }
                _ => return Err(Error::Malformed),
            };
            let modalities = match item.get("input_modalities") {
                None | Some(Value::Null) => None,
                Some(Value::Array(items))
                    if items.len() <= 16 && items.iter().all(|v| v.text().is_some()) =>
                {
                    Some(items)
                }
                _ => return Err(Error::Malformed),
            };
            let image = match modalities {
                Some(items) if items.iter().any(|value| value.text() == Some("image")) => {
                    Support::Supported
                }
                Some(_) => Support::Unsupported,
                None => Support::Unknown,
            };
            let visible = item.get("visibility").and_then(Value::text) == Some("list");
            let compatible = item.get("supports_reasoning_summary_parameter")
                != Some(&Value::Bool(false))
                && modalities
                    .is_none_or(|items| items.iter().any(|item| item.text() == Some("text")));
            let priority = item
                .get("priority")
                .and_then(|value| match value {
                    Value::Number(number) => number.parse::<i32>().ok(),
                    _ => None,
                })
                .unwrap_or(i32::MAX);
            entries.push(Entry {
                id: id.into(),
                name,
                default_effort,
                efforts,
                visible,
                compatible,
                image,
                priority,
            });
        }
        if entries.is_empty()
            || !entries
                .iter()
                .any(|entry| entry.visible && entry.compatible)
        {
            return Err(Error::Empty);
        }
        entries.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then(left.id.cmp(&right.id))
        });
        Ok(Self {
            entries,
            fetched_at: Instant::now(),
        })
    }

    pub fn fresh(&self) -> bool {
        self.fresh_at(Instant::now())
    }

    #[cfg(test)]
    pub(crate) fn stale_for_test(mut self) -> Self {
        self.fetched_at -= Duration::from_secs(301);
        self
    }

    fn fresh_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.fetched_at) < Duration::from_secs(300)
    }

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn support(&self, selection: Model) -> Support {
        let Some(entry) = self.entry(selection.id()) else {
            return Support::Unsupported;
        };
        if !entry.compatible {
            return Support::Unsupported;
        }
        let Some(effort) = selection.effort() else {
            return Support::Supported;
        };
        match &entry.efforts {
            Some(efforts) if efforts.iter().any(|level| level == effort) => Support::Supported,
            Some(_) => Support::Unsupported,
            None => Support::Unknown,
        }
    }

    pub fn image_support(&self, selection: Model) -> Support {
        self.entry(selection.id())
            .filter(|entry| entry.compatible)
            .map_or(Support::Unknown, |entry| entry.image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_distinct_efforts_defaults_and_unknown_metadata() {
        let catalog = Catalog::parse(br#"{"models":[{"slug":"model-a","display_name":"Alpha","visibility":"list","priority":2,"default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]},{"slug":"model-b","visibility":"list","priority":1}]}"#).unwrap();
        assert_eq!(catalog.entries[0].id, "model-b");
        assert_eq!(
            catalog.entry("model-a").unwrap().default_effort.as_deref(),
            Some("high")
        );
        assert_eq!(
            catalog.support(Model::new("model-a", Some("medium")).unwrap()),
            Support::Unsupported
        );
        assert_eq!(
            catalog.support(Model::new("model-b", Some("medium")).unwrap()),
            Support::Unknown
        );
        assert_eq!(
            catalog.support(Model::new("model-b", None).unwrap()),
            Support::Supported
        );
    }

    #[test]
    fn rejects_ambiguous_and_unbounded_catalogs() {
        for body in [
            r#"{"models":[]}"#,
            r#"{"models":[{"slug":"same","visibility":"list"},{"slug":"same","visibility":"list"}]}"#,
            r#"{"models":[{"slug":"bad\nvalue","visibility":"list"}]}"#,
            r#"{"models":[{"slug":"a","visibility":"list","supported_reasoning_levels":[{"effort":"low"},{"effort":"low"}]}]}"#,
        ] {
            assert!(Catalog::parse(body.as_bytes()).is_err());
        }
        assert_eq!(
            Catalog::parse(&vec![b' '; MAX_BYTES + 1]),
            Err(Error::Limit)
        );
    }

    #[test]
    fn stale_catalog_is_not_fresh_capability_evidence() {
        let catalog = Catalog::parse(br#"{"models":[{"slug":"model-a","visibility":"list","supported_reasoning_levels":[{"effort":"low"}]}]}"#).unwrap();
        assert!(catalog.fresh());
        assert!(catalog.fresh_at(catalog.fetched_at + Duration::from_secs(299)));
        assert!(!catalog.fresh_at(catalog.fetched_at + Duration::from_secs(301)));
    }

    #[test]
    fn image_support_requires_explicit_account_modalities() {
        let catalog = Catalog::parse(br#"{"models":[{"slug":"vision","visibility":"list","input_modalities":["text","image"]},{"slug":"text-only","visibility":"list","input_modalities":["text"]},{"slug":"unspecified","visibility":"list"}]}"#).unwrap();
        assert_eq!(
            catalog.image_support(Model::new("vision", None).unwrap()),
            Support::Supported
        );
        assert_eq!(
            catalog.image_support(Model::new("text-only", None).unwrap()),
            Support::Unsupported
        );
        assert_eq!(
            catalog.image_support(Model::new("unspecified", None).unwrap()),
            Support::Unknown
        );
        assert_eq!(
            catalog.image_support(Model::new("unknown", None).unwrap()),
            Support::Unknown
        );
    }
}
