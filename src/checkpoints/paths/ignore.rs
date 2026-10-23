//! Git ignore rules for implicit directory walks, read without running Git.
//! Walks test directories only: they skip a tree Git would not descend into,
//! while individually ignored files such as `.env` stay protected. Rules come
//! from `.gitignore` files and the repository's `info/exclude`; the user's
//! global excludes file is not read.
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
                load(&super::absolute(root, directory).join(".gitignore"))
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
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut parent = path;
        loop {
            parent = parent.rfind('/').map_or("", |at| &parent[..at]);
            let below = if parent.is_empty() {
                path
            } else {
                &path[parent.len() + 1..]
            };
            if let Some(ignored) = decide(&self.local(parent).rules, below, name) {
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
            decide(&rules.rules, &below, name)
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

fn decide(rules: &[Rule], below: &str, name: &str) -> Option<bool> {
    rules.iter().rev().find_map(|rule| {
        let text = if rule.anchored { below } else { name };
        matches(&rule.tokens, text.as_bytes()).then_some(!rule.negated)
    })
}

struct Rule {
    tokens: Vec<Token>,
    negated: bool,
    /// A rule with an inner or leading slash matches the path below its
    /// file's directory; any other rule matches a name at any depth.
    anchored: bool,
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
    // Walks test directories only, so a directory-only rule needs no flag.
    let pattern = pattern.strip_suffix('/').unwrap_or(pattern);
    let anchored = pattern.contains('/');
    let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
    (!pattern.is_empty()).then(|| Rule {
        tokens: tokens(pattern.as_bytes()),
        negated,
        anchored,
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
    let Some((token, rest)) = tokens.split_first() else {
        return text.is_empty();
    };
    let single = |accept: &dyn Fn(u8) -> bool| {
        text.first()
            .is_some_and(|byte| *byte != b'/' && accept(fold(*byte)))
            && matches(rest, &text[1..])
    };
    match token {
        Token::Byte(expected) => {
            text.first().is_some_and(|byte| fold(*byte) == *expected) && matches(rest, &text[1..])
        }
        Token::One => single(&|_| true),
        Token::Class { negated, ranges } => single(&|byte| {
            ranges
                .iter()
                .any(|(low, high)| (*low..=*high).contains(&byte))
                != *negated
        }),
        Token::Star => (0..=text.len())
            .take_while(|at| *at == 0 || text[at - 1] != b'/')
            .any(|at| matches(rest, &text[at..])),
        Token::Dirs => {
            matches(rest, text)
                || (0..text.len()).any(|at| text[at] == b'/' && matches(rest, &text[at + 1..]))
        }
        Token::All => true,
    }
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

#[cfg(test)]
#[path = "ignore_tests.rs"]
mod tests;
