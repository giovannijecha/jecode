//! Resolve restore requests against the saved spelling without treating a
//! different-case filesystem entry as a new path on case-insensitive volumes.
use super::{Index, recovery};
use crate::checkpoints::identity;
#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct TargetNames {
    root: PathBuf,
    recorded: HashMap<String, Vec<usize>>,
    catalog: Vec<String>,
    by_depth: HashMap<usize, Vec<usize>>,
    non_ascii_by_depth: HashMap<usize, Vec<usize>>,
    indexes: RefCell<HashMap<(usize, bool), DepthIndex>>,
    #[cfg(test)]
    index_probes: Cell<usize>,
    #[cfg(target_os = "macos")]
    directory_nodes: RefCell<DirectoryNodes>,
}

#[cfg(target_os = "macos")]
type MacNode = (u64, u64, u64, u64);
#[cfg(target_os = "macos")]
type DirectoryNodes = HashMap<(u64, u64), Option<HashMap<(u64, u64), usize>>>;

#[derive(Default)]
struct DepthIndex {
    spellings: HashMap<String, Vec<usize>>,
    #[cfg(target_os = "macos")]
    nodes: HashMap<MacNode, Vec<usize>>,
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
        let mut recorded: HashMap<String, Vec<usize>> = HashMap::new();
        let catalog: Vec<String> = names.into_iter().filter(|name| !name.is_empty()).collect();
        let mut by_depth: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut non_ascii_by_depth: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, name) in catalog.iter().enumerate() {
            recorded.entry(name.to_lowercase()).or_default().push(index);
            let depth = name.split('/').count();
            by_depth.entry(depth).or_default().push(index);
            if !name.is_ascii() {
                non_ascii_by_depth.entry(depth).or_default().push(index);
            }
        }
        Self {
            root: root.to_path_buf(),
            recorded,
            catalog,
            by_depth,
            non_ascii_by_depth,
            indexes: RefCell::new(HashMap::new()),
            #[cfg(test)]
            index_probes: Cell::new(0),
            #[cfg(target_os = "macos")]
            directory_nodes: RefCell::new(HashMap::new()),
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
            let sole_exact =
                bucket.is_some_and(|names| names.len() == 1 && self.catalog[names[0]] == prefix);
            if prefix.is_ascii()
                && !self.non_ascii_by_depth.contains_key(&count)
                && sole_exact
                && count == parts.len()
            {
                return Ok(prefix);
            }
            let mut candidates: Vec<usize> = bucket.into_iter().flatten().copied().collect();
            let mut seen: HashSet<usize> = candidates.iter().copied().collect();
            let mut node_hit = false;
            if ordinary(&identity::absolute(&self.root, &prefix)) {
                let (matches, hit) = self.matches(&prefix, count);
                node_hit = hit;
                for candidate in matches {
                    if seen.insert(candidate) {
                        candidates.push(candidate);
                    }
                }
            }
            if candidates.is_empty() {
                if !prefix.is_ascii() && !current_ordinary && uncertain.is_none() {
                    uncertain = self
                        .by_depth
                        .get(&count)
                        .filter(|names| !names.is_empty())
                        .map(|names| self.describe(names));
                }
                continue;
            }
            let proven: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|candidate| self.same_entry(&prefix, &self.catalog[*candidate]))
                .collect();
            if proven.len() > 1 {
                return Err(format!(
                    "Ambiguous restore path {raw:?}; recorded spellings are {}",
                    self.describe(&proven)
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
                    self.catalog[*candidate].clone()
                } else {
                    format!("{}/{suffix}", self.catalog[*candidate])
                });
            }
            if node_hit && current_ordinary {
                return Err(format!(
                    "Cannot verify restore path {raw:?}; recorded spellings sharing its file identity are {}",
                    self.describe(&candidates)
                ));
            }
            if candidates
                .iter()
                .any(|candidate| self.catalog[*candidate] == prefix)
            {
                if count == parts.len() && sole_exact {
                    return Ok(prefix);
                }
                if count == parts.len() && !current_ordinary && uncertain.is_none() {
                    uncertain = Some(self.describe(&candidates));
                }
            } else if uncertain.is_none() && (bucket.is_some() || !prefix.is_ascii()) {
                // A fallback candidate is only a positive identity probe for
                // an ASCII path. An unrelated Unicode name cannot make a
                // newly created and deleted ASCII path ambiguous.
                uncertain = Some(self.describe(&candidates));
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

    fn describe(&self, names: &[usize]) -> String {
        describe_candidates(
            names.iter().map(|name| self.catalog[*name].as_str()),
            names.len(),
        )
    }

    fn matches(&self, prefix: &str, depth: usize) -> (Vec<usize>, bool) {
        let unicode_only = prefix.is_ascii();
        let key = (depth, unicode_only);
        if !self.indexes.borrow().contains_key(&key) {
            let names = if unicode_only {
                self.non_ascii_by_depth.get(&depth)
            } else {
                self.by_depth.get(&depth)
            };
            let mut index = DepthIndex::default();
            for &candidate in names.into_iter().flatten() {
                #[cfg(test)]
                self.index_probes.set(self.index_probes.get() + 1);
                let path = identity::absolute(&self.root, &self.catalog[candidate]);
                if !ordinary(&path) {
                    continue;
                }
                if let Some(spelling) = identity::relative(&self.root, &path) {
                    index.spellings.entry(spelling).or_default().push(candidate);
                }
                #[cfg(target_os = "macos")]
                if let Some(node) = mac_node(&path) {
                    index.nodes.entry(node).or_default().push(candidate);
                }
            }
            self.indexes.borrow_mut().insert(key, index);
        }
        let path = identity::absolute(&self.root, prefix);
        let spelling = identity::relative(&self.root, &path);
        let indexes = self.indexes.borrow();
        let index = &indexes[&key];
        let matches = spelling
            .as_ref()
            .and_then(|spelling| index.spellings.get(spelling))
            .cloned()
            .unwrap_or_default();
        #[cfg(target_os = "macos")]
        {
            let mut matches = matches;
            let mut node_hit = false;
            if let Some(node) = mac_node(&path)
                && let Some(candidates) = index.nodes.get(&node)
            {
                node_hit = !candidates.is_empty();
                let mut seen: HashSet<usize> = matches.iter().copied().collect();
                for &candidate in candidates {
                    if seen.insert(candidate) {
                        matches.push(candidate);
                    }
                }
            }
            (matches, node_hit)
        }
        #[cfg(not(target_os = "macos"))]
        {
            (matches, false)
        }
    }

    #[cfg(test)]
    pub(crate) fn index_probes(&self) -> usize {
        self.index_probes.get()
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
            self.same_macos_entry(&current_path, &saved_path)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    // realpath may preserve an alias's Unicode spelling on APFS. Count the
    // actual entries once; distinct hard links must stay separate.
    #[cfg(target_os = "macos")]
    fn same_macos_entry(&self, current: &Path, saved: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;

        let Some(node) = mac_node(current) else {
            return false;
        };
        if Some(node) != mac_node(saved) {
            return false;
        }
        let parent = (node.0, node.1);
        if !self.directory_nodes.borrow().contains_key(&parent) {
            let counts = current.parent().and_then(|path| {
                let mut counts = HashMap::new();
                for entry in fs::read_dir(path).ok()? {
                    let metadata = fs::symlink_metadata(entry.ok()?.path()).ok()?;
                    *counts.entry((metadata.dev(), metadata.ino())).or_insert(0) += 1;
                }
                Some(counts)
            });
            self.directory_nodes.borrow_mut().insert(parent, counts);
        }
        self.directory_nodes
            .borrow()
            .get(&parent)
            .and_then(Option::as_ref)
            .and_then(|counts| counts.get(&(node.2, node.3)))
            == Some(&1)
    }
}

fn describe_candidates<'a>(names: impl IntoIterator<Item = &'a str>, total: usize) -> String {
    const SHOWN: usize = 4;
    let shown: Vec<&str> = names.into_iter().take(SHOWN).collect();
    let omitted = total.saturating_sub(shown.len());
    let mut result = shown.join(", ");
    if omitted != 0 {
        result.push_str(&format!(" (+{omitted} more)"));
    }
    result
}

#[cfg(target_os = "macos")]
fn mac_node(path: &Path) -> Option<MacNode> {
    use std::os::unix::fs::MetadataExt;

    let parent = fs::metadata(path.parent()?).ok()?;
    let node = fs::symlink_metadata(path).ok()?;
    Some((parent.dev(), parent.ino(), node.dev(), node.ino()))
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
