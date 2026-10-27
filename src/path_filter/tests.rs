use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "jecode-ignore-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join(".git/info")).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pattern(pattern: &str, text: &str) -> bool {
    let rule = rule(pattern).unwrap();
    let name = text.rsplit('/').next().unwrap();
    decide(&[rule], text, name) == Some(true)
}

#[test]
fn patterns_follow_gitignore_syntax() {
    assert!(pattern("target", "crates/app/target"));
    assert!(pattern("/target/", "target"));
    assert!(!pattern("/target/", "crates/target"));
    assert!(pattern("build-*", "web/build-cache"));
    assert!(!pattern("docs/*", "docs/api/v1"));
    assert!(pattern("docs/*", "docs/api"));
    assert!(pattern("**/cache", "a/b/cache"));
    assert!(pattern("**/cache", "cache"));
    assert!(pattern("a/**/z", "a/z"));
    assert!(pattern("a/**/z", "a/b/c/z"));
    assert!(pattern("out/**", "out/x/y"));
    assert!(!pattern("out/**", "out"));
    assert!(pattern("tmp[0-9]", "tmp7"));
    assert!(!pattern("tmp[!0-9]", "tmp7"));
    assert!(pattern("v?", "v1"));
    assert!(pattern(r"\#literal", "#literal"));
    assert!(pattern(r"trailing\ ", "trailing "));
    assert!(pattern("spaces   ", "spaces"));
    assert!(rule("# comment").is_none());
    assert!(rule("").is_none());
    assert!(!pattern("!kept", "kept"));
}

#[test]
fn many_wildcards_do_not_backtrack_and_explicit_matching_is_cancellable() {
    let pattern = Pattern::new(&format!("{}b", "*a".repeat(40)));
    let cancellation = crate::cancel::Cancellation::default();
    assert!(
        !pattern
            .matches(&format!("{}c", "a".repeat(100)), &cancellation)
            .unwrap()
    );
    assert!(
        pattern
            .matches(&format!("{}b", "a".repeat(100)), &cancellation)
            .unwrap()
    );
    cancellation.cancel();
    assert!(
        pattern
            .matches("ab", &cancellation)
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn discovery_directory_only_rules_do_not_ignore_files_of_the_same_name() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "cache/\n*.log\n");
    let mut rules = Ignore::new(&fixture.0);
    assert!(rules.ignored_entry("cache", true));
    assert!(!rules.ignored_entry("cache", false));
    assert!(rules.ignored_entry("out.log", false));
}

#[test]
fn nearest_file_and_last_rule_decide() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "build/\n.tmp/\nlogs/\n!logs/\n");
    fixture.write("web/.gitignore", "!build/\ndist\n");
    fixture.write(".git/info/exclude", "scratch/\n");
    let mut rules = Ignore::new(&fixture.0);
    assert!(rules.ignored(".tmp"));
    assert!(rules.ignored("build"));
    assert!(!rules.ignored("logs"));
    assert!(!rules.ignored("web/build"));
    assert!(rules.ignored("web/dist"));
    assert!(rules.ignored("scratch"));
    assert!(rules.ignored("web/scratch"));
    assert!(!rules.ignored("src"));
}

#[test]
fn repository_rules_above_the_workspace_apply() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "/app/generated/\ncache/\n");
    fixture.write("app/src/.keep", "");
    let mut rules = Ignore::new(&fixture.0.join("app"));
    assert!(rules.ignored("generated"));
    assert!(rules.ignored("src/cache"));
    assert!(!rules.ignored("src"));
}

#[test]
fn worktree_rules_come_from_the_common_directory() {
    let main = Fixture::new();
    main.write(".git/info/exclude", "local/\n");
    main.write(".git/worktrees/feature/commondir", "../..\n");
    let worktree = Fixture::new();
    fs::remove_dir_all(worktree.0.join(".git")).unwrap();
    worktree.write(
        ".git",
        &format!(
            "gitdir: {}\n",
            main.0.join(".git/worktrees/feature").display()
        ),
    );
    let mut rules = Ignore::new(&worktree.0);
    assert!(rules.ignored("local"));
    assert!(!rules.ignored("src"));
}
