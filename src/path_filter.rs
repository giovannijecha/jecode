//! Shared Git-style path filtering, read without running Git.
//! Checkpoints test directories only; discovery can also filter individual files.
//! Rules come from `.gitignore` files and the repository's `info/exclude`;
//! the user's global excludes file is not read.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct IgnoreRules {
    pub local: BTreeMap<String, String>,
    pub outer: Vec<(String, String)>,
}

#[derive(Default)]
struct RuleFile {
    text: String,
    rules: Vec<Rule>,
}

impl RuleFile {
    fn parse(text: String) -> Self {
        let rules = text.lines().filter_map(rule).collect();
        Self { text, rules }
    }
}

pub struct Ignore {
    /// Present during capture; restored rules never read the live files.
    root: Option<PathBuf>,
    /// `.gitignore` rules of workspace directories, read on first use.
    local: HashMap<String, RuleFile>,
    /// Repository rules outside the workspace, nearest first, each with the
    /// workspace's path relative to the directory that owns them.
    outer: Vec<(String, RuleFile)>,
}

impl Ignore {
    pub fn new(root: &Path) -> Self {
        let mut outer = Vec::new();
        let mut prefix = String::new();
        let mut directory = root.to_path_buf();
        loop {
            if !prefix.is_empty() {
                outer.push((prefix.clone(), load(&directory.join(".gitignore"))));
            }
            if let Some(exclude) = exclude_file(&directory) {
                outer.push((prefix, load(&exclude)));
                break;
            }
            let name = directory.file_name().and_then(|name| name.to_str());
            let Some(name) = name.map(str::to_owned) else {
                // No repository owns this workspace; only its own files apply.
                outer.clear();
                break;
            };
            prefix = if prefix.is_empty() {
                name
            } else {
                format!("{name}/{prefix}")
            };
            directory.pop();
        }
        Self {
            root: Some(root.to_path_buf()),
            local: HashMap::new(),
            outer,
        }
    }

    pub fn saved(rules: &IgnoreRules) -> Self {
        Self {
            root: None,
            local: rules
                .local
                .iter()
                .map(|(path, text)| (path.clone(), RuleFile::parse(text.clone())))
                .collect(),
            outer: rules
                .outer
                .iter()
                .map(|(prefix, text)| (prefix.clone(), RuleFile::parse(text.clone())))
                .collect(),
        }
    }

    fn local(&mut self, directory: &str) -> &RuleFile {
        self.local.entry(directory.to_owned()).or_insert_with(|| {
            self.root.as_ref().map_or_else(RuleFile::default, |root| {
                load(&root.join(directory).join(".gitignore"))
            })
        })
    }

    /// Keep the rule sources, including ancestors of an explicitly selected
    /// or absent directory. They define eligibility after the live rules change.
    pub fn snapshot(mut self, roots: &BTreeSet<String>) -> IgnoreRules {
        for root in roots {
            let mut directory = root.as_str();
            loop {
                self.local(directory);
                if directory.is_empty() {
                    break;
                }
                directory = directory.rfind('/').map_or("", |at| &directory[..at]);
            }
        }
        IgnoreRules {
            local: self
                .local
                .into_iter()
                .filter(|(_, file)| !file.rules.is_empty())
                .map(|(path, file)| (path, file.text))
                .collect(),
            outer: self
                .outer
                .into_iter()
                .filter(|(_, file)| !file.rules.is_empty())
                .map(|(prefix, file)| (prefix, file.text))
                .collect(),
        }
    }

    /// Records whether a walked directory holds a `.gitignore`, sparing a
    /// failed open for each directory without one.
    pub fn visit(&mut self, directory: &str, has_rules: bool) {
        if has_rules {
            self.local(directory);
        } else {
            self.local.entry(directory.to_owned()).or_default();
        }
    }

    /// Whether Git ignores this workspace-relative directory, assuming its
    /// ancestors were traversed. The nearest rule file decides first, and
    /// within one file the last matching rule wins.
    pub fn ignored(&mut self, path: &str) -> bool {
        self.ignored_entry(path, true)
    }

    /// Whether this entry is ignored, after its parents have been traversed.
    pub fn ignored_entry(&mut self, path: &str, directory: bool) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut parent = path;
        loop {
            parent = parent.rfind('/').map_or("", |at| &parent[..at]);
            let below = if parent.is_empty() {
                path
            } else {
                &path[parent.len() + 1..]
            };
            if let Some(ignored) = decide_entry(&self.local(parent).rules, below, name, directory) {
                return ignored;
            }
            if parent.is_empty() {
                break;
            }
        }
        self.outer.iter().find_map(|(prefix, rules)| {
            let below = if prefix.is_empty() {
                path.to_owned()
            } else {
                format!("{prefix}/{path}")
            };
            decide_entry(&rules.rules, &below, name, directory)
        }) == Some(true)
    }

    /// An explicit scope bypasses ignore rules for itself and its ancestors,
    /// while its implicit descendants still follow the captured rules.
    pub fn ignored_beneath(&mut self, path: &str, scope: &str, directory: bool) -> bool {
        let mut ancestor = if directory {
            path
        } else {
            path.rfind('/').map_or("", |at| &path[..at])
        };
        while ancestor.len() > scope.len() {
            if self.ignored(ancestor) {
                return true;
            }
            ancestor = ancestor.rfind('/').map_or("", |at| &ancestor[..at]);
        }
        false
    }
}

/// The `info/exclude` file of a repository rooted here, for a `.git`
/// directory or a worktree's `gitdir:` file.
fn exclude_file(directory: &Path) -> Option<PathBuf> {
    let marker = directory.join(".git");
    let metadata = fs::metadata(&marker).ok()?;
    let git = if metadata.is_dir() {
        marker
    } else {
        let text = fs::read_to_string(&marker).ok()?;
        let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
        let git = join(directory, target);
        match fs::read_to_string(git.join("commondir")) {
            Ok(common) => join(&git, common.trim()),
            Err(_) => git,
        }
    };
    Some(git.join("info").join("exclude"))
}

/// Resolves `..` lexically: verbatim Windows paths leave it to the caller.
fn join(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            other => path.push(other),
        }
    }
    path
}

fn load(path: &Path) -> RuleFile {
    fs::read(path)
        .map(|bytes| RuleFile::parse(String::from_utf8_lossy(&bytes).into_owned()))
        .unwrap_or_default()
}

#[cfg(test)]
fn decide(rules: &[Rule], below: &str, name: &str) -> Option<bool> {
    decide_entry(rules, below, name, true)
}

fn decide_entry(rules: &[Rule], below: &str, name: &str, directory: bool) -> Option<bool> {
    rules.iter().rev().find_map(|rule| {
        let text = if rule.anchored { below } else { name };
        ((!rule.directory_only || directory) && matches(&rule.tokens, text.as_bytes()))
            .then_some(!rule.negated)
    })
}

struct Rule {
    tokens: Vec<Token>,
    negated: bool,
    /// A rule with an inner or leading slash matches the path below its
    /// file's directory; any other rule matches a name at any depth.
    anchored: bool,
    directory_only: bool,
}

enum Token {
    Byte(u8),
    One,
    Star,
    /// `**/`: zero or more whole directories.
    Dirs,
    /// A trailing `/**` or a lone `**`: everything that remains.
    All,
    Class {
        negated: bool,
        ranges: Vec<(u8, u8)>,
    },
}

fn rule(line: &str) -> Option<Rule> {
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let bytes = line.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b' ' && (end < 2 || bytes[end - 2] != b'\\') {
        end -= 1;
    }
    let mut pattern = &line[..end];
    let negated = pattern.starts_with('!');
    if negated {
        pattern = &pattern[1..];
    }
    let directory_only = pattern.ends_with('/');
    let pattern = pattern.strip_suffix('/').unwrap_or(pattern);
    let anchored = pattern.contains('/');
    let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
    (!pattern.is_empty()).then(|| Rule {
        tokens: tokens(pattern.as_bytes()),
        negated,
        anchored,
        directory_only,
    })
}

fn tokens(pattern: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < pattern.len() {
        let byte = pattern[at];
        at += 1;
        match byte {
            b'\\' if at < pattern.len() => {
                tokens.push(Token::Byte(fold(pattern[at])));
                at += 1;
            }
            b'?' => tokens.push(Token::One),
            b'*' => {
                let start = at - 1;
                while pattern.get(at) == Some(&b'*') {
                    at += 1;
                }
                let segment = at - start > 1 && (start == 0 || pattern[start - 1] == b'/');
                if segment && at == pattern.len() {
                    tokens.push(Token::All);
                } else if segment && pattern[at] == b'/' {
                    at += 1;
                    tokens.push(Token::Dirs);
                } else {
                    tokens.push(Token::Star);
                }
            }
            b'[' => match class(&pattern[at..]) {
                Some((token, used)) => {
                    tokens.push(token);
                    at += used;
                }
                None => tokens.push(Token::Byte(b'[')),
            },
            byte => tokens.push(Token::Byte(fold(byte))),
        }
    }
    tokens
}

/// Parses a bracket expression after its `[`, returning the bytes consumed.
fn class(pattern: &[u8]) -> Option<(Token, usize)> {
    let negated = matches!(pattern.first(), Some(b'!' | b'^'));
    let mut at = usize::from(negated);
    let mut ranges = Vec::new();
    loop {
        let mut low = *pattern.get(at)?;
        if low == b']' && !ranges.is_empty() {
            return Some((Token::Class { negated, ranges }, at + 1));
        }
        at += 1;
        if low == b'\\' {
            low = *pattern.get(at)?;
            at += 1;
        }
        let mut high = low;
        if pattern.get(at) == Some(&b'-') && pattern.get(at + 1).is_some_and(|byte| *byte != b']') {
            high = pattern[at + 1];
            at += 2;
            if high == b'\\' {
                high = *pattern.get(at)?;
                at += 1;
            }
        }
        ranges.push((fold(low), fold(high)));
    }
}

fn matches(tokens: &[Token], text: &[u8]) -> bool {
    match_tokens(tokens, text, None).unwrap()
}

/// Reachable byte positions after each token avoid exponential star backtracking.
fn match_tokens(
    tokens: &[Token],
    text: &[u8],
    cancellation: Option<&crate::cancel::Cancellation>,
) -> Result<bool, String> {
    if cancellation.is_some_and(crate::cancel::Cancellation::requested) {
        return Err("Operation cancelled".into());
    }
    if tokens.iter().all(|token| matches!(token, Token::Byte(_))) {
        return Ok(tokens.len() == text.len()
            && tokens.iter().zip(text).all(
                |(token, byte)| matches!(token, Token::Byte(expected) if *expected == fold(*byte)),
            ));
    }
    let mut current = vec![false; text.len() + 1];
    let mut next = current.clone();
    current[0] = true;
    for token in tokens {
        if cancellation.is_some_and(crate::cancel::Cancellation::requested) {
            return Err("Operation cancelled".into());
        }
        next.fill(false);
        match token {
            Token::All => return Ok(current.iter().any(|matched| *matched)),
            Token::Star => {
                next[0] = current[0];
                for index in 0..text.len() {
                    next[index + 1] = current[index + 1] || (next[index] && text[index] != b'/');
                }
            }
            Token::Dirs => {
                next.copy_from_slice(&current);
                let mut active = false;
                for index in 0..text.len() {
                    active |= current[index];
                    if active && text[index] == b'/' {
                        next[index + 1] = true;
                    }
                }
            }
            _ => {
                for (index, byte) in text.iter().copied().enumerate() {
                    if !current[index] {
                        continue;
                    }
                    let accepted = match token {
                        Token::Byte(expected) => fold(byte) == *expected,
                        Token::One => byte != b'/',
                        Token::Class { negated, ranges } => {
                            byte != b'/'
                                && (ranges
                                    .iter()
                                    .any(|(low, high)| (*low..=*high).contains(&fold(byte)))
                                    != *negated)
                        }
                        _ => unreachable!(),
                    };
                    next[index + 1] = accepted;
                }
            }
        }
        std::mem::swap(&mut current, &mut next);
    }
    Ok(current[text.len()])
}

/// Git defaults to case-insensitive matching on Windows volumes.
fn fold(byte: u8) -> u8 {
    #[cfg(windows)]
    {
        byte.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        byte
    }
}

/// An explicit path glob. Unlike ignore rules, `!` and `#` are literal here.
pub struct Pattern {
    tokens: Vec<Token>,
    anchored: bool,
}

impl Pattern {
    pub fn new(pattern: &str) -> Self {
        Self {
            tokens: tokens(pattern.trim_start_matches('/').as_bytes()),
            anchored: pattern.contains('/'),
        }
    }

    pub fn matches(
        &self,
        relative: &str,
        cancellation: &crate::cancel::Cancellation,
    ) -> Result<bool, String> {
        let text = if self.anchored {
            relative
        } else {
            relative.rsplit('/').next().unwrap_or(relative)
        };
        match_tokens(&self.tokens, text.as_bytes(), Some(cancellation))
    }
}

#[cfg(test)]
mod tests;
