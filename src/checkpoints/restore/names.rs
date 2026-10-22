//! Resolve restore requests against the saved spelling without treating a
//! different-case filesystem entry as a new path on case-insensitive volumes.
use super::{Index, recovery};
use crate::checkpoints::identity;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct TargetNames {
    root: PathBuf,
    recorded: HashMap<String, Vec<String>>,
    catalog: Vec<String>,
    non_ascii: Vec<String>,
    non_ascii_depths: HashSet<usize>,
}

impl TargetNames {
    pub(crate) fn new(root: &Path, index: &Index, target: u64) -> Self {
        let mut names = BTreeSet::new();
        if super::recovery_reference(index, target) {
            names.extend(recovery::paths(index, target));
            names.extend(recovery::roots(index, target));
        } else {
            names.extend(
                index
                    .paths
                    .keys()
                    .filter(|path| {
                        index
                            .at(path, target)
                            .is_some_and(|version| version.state.exists())
                    })
                    .cloned(),
            );
        }
        let mut recorded: HashMap<String, Vec<String>> = HashMap::new();
        let catalog: Vec<String> = names.into_iter().filter(|name| !name.is_empty()).collect();
        let mut non_ascii = Vec::new();
        let mut non_ascii_depths = HashSet::new();
        for name in &catalog {
            recorded
                .entry(name.to_lowercase())
                .or_default()
                .push(name.clone());
            if !name.is_ascii() {
                non_ascii_depths.insert(name.split('/').count());
                non_ascii.push(name.clone());
            }
        }
        Self {
            root: root.to_path_buf(),
            recorded,
            catalog,
            non_ascii,
            non_ascii_depths,
        }
    }

    pub(crate) fn resolve(&self, raw: &str) -> Result<String, String> {
        let normalized = super::normalize(raw)?;
        let current = self.spelling(&normalized);
        let current_path = identity::absolute(&self.root, &current);
        let current_ordinary = ordinary(&current_path);
        let parts: Vec<&str> = current.split('/').collect();
        let mut uncertain: Option<String> = None;
        for count in (1..=parts.len()).rev() {
            let prefix = parts[..count].join("/");
            let bucket = self.recorded.get(&prefix.to_lowercase());
            let sole_exact = bucket.is_some_and(|names| names.len() == 1 && names[0] == prefix);
            if prefix.is_ascii()
                && !self.non_ascii_depths.contains(&count)
                && sole_exact
                && count == parts.len()
            {
                return Ok(prefix);
            }
            let mut candidates: Vec<&str> =
                bucket.into_iter().flatten().map(String::as_str).collect();
            let fallback = if prefix.is_ascii() {
                &self.non_ascii
            } else {
                &self.catalog
            };
            for candidate in fallback {
                if candidate.split('/').count() == count
                    && !candidates.contains(&candidate.as_str())
                {
                    candidates.push(candidate);
                }
            }
            if candidates.is_empty() {
                continue;
            }
            let proven: Vec<&str> = candidates
                .iter()
                .copied()
                .filter(|candidate| self.same_entry(&prefix, candidate))
                .collect();
            if proven.len() > 1 {
                return Err(format!(
                    "Ambiguous restore path {raw:?}; recorded spellings are {}",
                    describe_candidates(candidates.iter().copied())
                ));
            }
            if let Some(candidate) = proven.first() {
                if let Some(candidates) = &uncertain
                    && !current_ordinary
                {
                    return Err(format!(
                        "Cannot verify restore path {raw:?}; use the exact recorded spelling {}",
                        candidates
                    ));
                }
                let suffix = parts[count..].join("/");
                return Ok(if suffix.is_empty() {
                    (*candidate).to_string()
                } else {
                    format!("{candidate}/{suffix}")
                });
            }
            if candidates.contains(&prefix.as_str()) {
                if count == parts.len() && sole_exact {
                    return Ok(prefix);
                }
                if count == parts.len() && !current_ordinary && uncertain.is_none() {
                    uncertain = Some(describe_candidates(candidates.iter().copied()));
                }
            } else if uncertain.is_none() && (bucket.is_some() || !prefix.is_ascii()) {
                // A fallback candidate is only a positive identity probe for
                // an ASCII path. An unrelated Unicode name cannot make a
                // newly created and deleted ASCII path ambiguous.
                uncertain = Some(describe_candidates(candidates.iter().copied()));
            }
        }
        if let Some(candidates) = &uncertain
            && !current_ordinary
        {
            return Err(format!(
                "Cannot verify restore path {raw:?}; use the exact recorded spelling {}",
                candidates
            ));
        }
        Ok(current)
    }

    fn spelling(&self, path: &str) -> String {
        identity::relative(&self.root, &identity::absolute(&self.root, path))
            .unwrap_or_else(|| path.to_string())
    }

    fn same_entry(&self, current: &str, saved: &str) -> bool {
        let current_path = identity::absolute(&self.root, current);
        let saved_path = identity::absolute(&self.root, saved);
        if !ordinary(&current_path) || !ordinary(&saved_path) {
            return false;
        }
        if identity::relative(&self.root, &current_path)
            .zip(identity::relative(&self.root, &saved_path))
            .is_some_and(|(current, saved)| current == saved)
        {
            return true;
        }
        #[cfg(target_os = "macos")]
        {
            return same_macos_entry(&current_path, &saved_path);
        }
        #[cfg(not(target_os = "macos"))]
        false
    }
}

fn describe_candidates<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    const SHOWN: usize = 4;
    let mut shown = Vec::new();
    let mut omitted = 0usize;
    for name in names {
        if shown.len() < SHOWN {
            shown.push(name);
        } else {
            omitted += 1;
        }
    }
    let mut result = shown.join(", ");
    if omitted != 0 {
        result.push_str(&format!(" (+{omitted} more)"));
    }
    result
}

// realpath may preserve an alias's Unicode spelling on APFS. A unique entry
// with the same file ID in the same directory proves the two paths name one
// entry; distinct hard links must not be coalesced.
#[cfg(target_os = "macos")]
fn same_macos_entry(current: &Path, saved: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Some((current_parent, saved_parent)) = current.parent().zip(saved.parent()) else {
        return false;
    };
    let Ok(current_dir) = fs::metadata(current_parent) else {
        return false;
    };
    let Ok(saved_dir) = fs::metadata(saved_parent) else {
        return false;
    };
    if (current_dir.dev(), current_dir.ino()) != (saved_dir.dev(), saved_dir.ino()) {
        return false;
    }
    let Ok(current_node) = fs::symlink_metadata(current) else {
        return false;
    };
    let Ok(saved_node) = fs::symlink_metadata(saved) else {
        return false;
    };
    let node = (current_node.dev(), current_node.ino());
    if node != (saved_node.dev(), saved_node.ino()) {
        return false;
    }
    let Ok(entries) = fs::read_dir(current_parent) else {
        return false;
    };
    let mut matching = 0;
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if (metadata.dev(), metadata.ino()) == node {
            matching += 1;
            if matching > 1 {
                return false;
            }
        }
    }
    matching == 1
}

fn ordinary(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return false;
        }
    }
    metadata.is_file() || metadata.is_dir()
}
