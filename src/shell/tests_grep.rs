use super::tests::shell;
use super::tests_grep_syntax::over;
use super::tests_runtime::bounded_script;
use super::*;

fn read(directory: &crate::test_support::Directory, name: &str) -> String {
    fs::read_to_string(directory.path().join(name)).unwrap()
}

fn grep(script: &str) -> (i32, String) {
    let (directory, mut shell) = shell();
    fs::write(
        directory.path().join("log"),
        "ok 1 - parses\nnot ok 2 - rounds\n# pass 1\n# fail 1\nℹ pass 10\r\nfn main() {}\nERROR: a+b (x)\n",
    )
    .unwrap();
    let status = shell.run(&format!("{script} > out"));
    (status, read(&directory, "out"))
}

#[test]
fn grep_selects_lines_with_extended_and_basic_patterns() {
    for (script, expected) in [
        ("grep -E '^# (pass|fail)' log", "# pass 1\n# fail 1\n"),
        ("grep -E \"^ℹ (pass|fail)\" log", "ℹ pass 10\n"),
        ("grep '^# \\(pass\\|fail\\)' log", "# pass 1\n# fail 1\n"),
        ("grep 'a+b (x)' log", "ERROR: a+b (x)\n"),
        ("grep -F 'a+b (x)' log", "ERROR: a+b (x)\n"),
        (
            "grep -E 'ok [0-9]+ -' log",
            "ok 1 - parses\nnot ok 2 - rounds\n",
        ),
        (
            "grep -Ei 'error|^NOT' log",
            "not ok 2 - rounds\nERROR: a+b (x)\n",
        ),
        ("grep -w ok log", "ok 1 - parses\nnot ok 2 - rounds\n"),
        ("grep -x 'fn main() {}' log", "fn main() {}\n"),
        ("grep -E '\\bmain\\(\\)' log", "fn main() {}\n"),
        ("grep -E '[[:digit:]]{2}' log", "ℹ pass 10\n"),
        (
            "grep -E '\\d\\s-\\s\\w+s$' log",
            "ok 1 - parses\nnot ok 2 - rounds\n",
        ),
        ("grep -cv ok log", "5\n"),
        ("grep -n -m1 pass log", "3:# pass 1\n"),
        ("grep -o -E '[0-9]+' log", "1\n2\n1\n1\n10\n"),
        (
            "grep -e parses -e rounds log",
            "ok 1 - parses\nnot ok 2 - rounds\n",
        ),
        (
            "grep -A1 -n rounds log",
            "2:not ok 2 - rounds\n3-# pass 1\n",
        ),
        ("grep -B1 main log", "ℹ pass 10\nfn main() {}\n"),
        (
            "grep -C1 -E 'parses|main' log",
            "ok 1 - parses\nnot ok 2 - rounds\n--\nℹ pass 10\nfn main() {}\nERROR: a+b (x)\n",
        ),
        ("cat log | grep -c '#'", "2\n"),
        ("grep -q rounds log && echo found", "found\n"),
    ] {
        let (status, output) = grep(script);
        assert_eq!((status, output.as_str()), (0, expected), "{script}");
    }
    assert_eq!(grep("grep absent log"), (1, String::new()));
    assert_eq!(grep("grep -E '(' log").0, 2);
}

#[test]
fn grep_searches_trees_with_names_and_filters() {
    let (directory, mut shell) = shell();
    let script = "mkdir -p src/deep node_modules/pkg; \
        echo 'let target = 1;' > src/a.rs; echo 'target' > src/b.txt; \
        echo 'let target = 2;' > src/deep/c.rs; echo target > node_modules/pkg/d.rs; \
        grep -rn target src > tree; grep -rl target --include='*.rs' --exclude-dir=node_modules . > listed; \
        cd src && grep -r target > ../implicit; cd ..; \
        grep -h target src/a.rs src/b.txt > bare; grep -L absent src/a.rs > without; \
        grep target src missing 2> errors; echo \"$?\" > status";
    assert_eq!(shell.run(script), 0);
    assert_eq!(
        read(&directory, "tree"),
        "src/a.rs:1:let target = 1;\nsrc/b.txt:1:target\nsrc/deep/c.rs:1:let target = 2;\n"
    );
    assert_eq!(read(&directory, "listed"), "./src/a.rs\n./src/deep/c.rs\n");
    assert_eq!(
        read(&directory, "implicit"),
        "a.rs:let target = 1;\nb.txt:target\ndeep/c.rs:let target = 2;\n"
    );
    assert_eq!(read(&directory, "bare"), "let target = 1;\ntarget\n");
    assert_eq!(read(&directory, "without"), "src/a.rs\n");
    let errors = read(&directory, "errors");
    assert!(
        errors.contains("src: Is a directory") && errors.contains("missing"),
        "{errors}"
    );
    assert_eq!(read(&directory, "status"), "2\n");
}

/// A link to a directory: a symbolic link on Unix, a junction on Windows.
pub(super) fn directory_link(target: &std::path::Path, link: &std::path::Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        // `cmd` would read a forward slash as a switch.
        let native = |path: &std::path::Path| path.display().to_string().replace('/', "\\");
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(native(link))
            .arg(native(target))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}

#[test]
fn recursive_search_follows_links_only_when_named() {
    let directory = crate::test_support::Directory::new();
    let root = directory.path().join("tree");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("a"), "hit\n").unwrap();
    fs::write(root.join("sub/b"), "hit\n").unwrap();
    directory_link(&root.join("sub"), &root.join("link"));
    directory_link(&root, &root.join("sub/up"));
    #[cfg(unix)]
    std::os::unix::fs::symlink("a", root.join("file_link")).unwrap();
    let script = "grep -r hit tree > walked; echo \"$?\" > walked_status; \
        grep -r hit tree/link > named; \
        grep -R hit tree > followed; echo \"$?\" > followed_status";
    assert_eq!(bounded_script(script, &directory), 0);
    assert_eq!(read(&directory, "walked"), "tree/a:hit\ntree/sub/b:hit\n");
    assert_eq!(read(&directory, "walked_status"), "0\n");
    assert_eq!(read(&directory, "named"), "tree/link/b:hit\n");
    let followed = read(&directory, "followed");
    for line in ["tree/a:hit", "tree/link/b:hit", "tree/sub/b:hit"] {
        assert!(followed.lines().any(|found| found == line), "{followed}");
    }
    assert_eq!(read(&directory, "followed_status"), "0\n");
}

#[cfg(windows)]
#[test]
fn recursive_search_skips_a_dangling_junction() {
    let (directory, mut shell) = shell();
    let root = directory.path().join("tree");
    fs::create_dir_all(root.join("gone")).unwrap();
    fs::write(root.join("a"), "hit\n").unwrap();
    directory_link(&root.join("gone"), &root.join("link"));
    fs::remove_dir(root.join("gone")).unwrap();
    assert_eq!(shell.run("grep -r hit tree > walked"), 0);
    assert_eq!(read(&directory, "walked"), "tree/a:hit\n");
}

#[test]
fn binary_input_follows_the_chosen_mode() {
    let input = "a\0b\n";
    for (args, status, output) in [
        ("a", 0, "Binary file in matches\n"),
        ("-c a", 0, "1\n"),
        ("-l a", 0, "in\n"),
        ("-a a", 0, "a\0b\n"),
        ("--text a", 0, "a\0b\n"),
        ("--binary-files=text a", 0, "a\0b\n"),
        ("--binary-files binary a", 0, "Binary file in matches\n"),
        ("-av zz", 0, "a\0b\n"),
        ("-aI a", 1, ""),
        ("-I a", 1, ""),
        ("--binary-files=without-match a", 1, ""),
        ("-Ic a", 1, "0\n"),
        ("-IL a", 1, "in\n"),
        ("-Ia a", 0, "a\0b\n"),
        ("--binary-files=other a", 2, ""),
    ] {
        assert_eq!(
            over(input, args),
            (status, output.to_owned()),
            "grep {args}"
        );
    }
}

#[test]
fn binary_input_splits_at_nul_and_stops_at_its_first_match() {
    for (input, args, status, output) in [
        ("a\0a\nb\n", "-c a", 0, "2\n"),
        ("foo\0bar\n", "-c '^bar'", 0, "1\n"),
        ("a\0b\n", "-cv zz", 0, "2\n"),
        ("a\0a\n", "-ac a", 0, "1\n"),
        ("x\nfoo\0bar\n", "x", 0, "Binary file in matches\n"),
        ("x\nfoo\0bar\n", "-I x", 1, ""),
        ("\0\0\0\nhit\0\0\n", "-c hit", 0, "1\n"),
        ("\0\0\0", "-c ''", 0, "3\n"),
    ] {
        assert_eq!(
            over(input, args),
            (status, output.to_owned()),
            "grep {args} over {input:?}"
        );
    }
    let directory = crate::test_support::Directory::new();
    fs::write(directory.path().join("binary"), "a\0b\n").unwrap();
    let script = "while true; do cat binary; done | grep a > first; echo after > marker";
    assert_eq!(bounded_script(script, &directory), 0);
    assert_eq!(
        read(&directory, "first"),
        "Binary file (standard input) matches\n"
    );
    assert_eq!(read(&directory, "marker"), "after\n");
}

#[test]
fn grep_writes_the_original_bytes_of_a_line() {
    let (directory, mut shell) = shell();
    fs::write(
        directory.path().join("latin1"),
        b"caf\xe9 foo\nna\xefve\n\xff\xfe\n",
    )
    .unwrap();
    fs::write(directory.path().join("utf8"), "ℹxé\n").unwrap();
    let script = "grep -v foo latin1 > kept; grep -o 'caf.' latin1 > only; \
        grep -n ve latin1 > numbered; grep -B1 ve latin1 > context; \
        grep -A1 foo latin1 > after; grep -o . utf8 > characters; \
        grep -o 'x*' utf8 > empty_steps; grep -w 'é' utf8 > word; \
        grep -o '\\bx' latin1 utf8 > boundary";
    shell.run(script);
    let bytes = |name: &str| fs::read(directory.path().join(name)).unwrap();
    assert_eq!(bytes("kept"), b"na\xefve\n\xff\xfe\n");
    assert_eq!(bytes("only"), b"caf\xe9\n");
    assert_eq!(bytes("numbered"), b"2:na\xefve\n");
    assert_eq!(bytes("context"), b"caf\xe9 foo\nna\xefve\n");
    assert_eq!(bytes("after"), b"caf\xe9 foo\nna\xefve\n");
    assert_eq!(bytes("characters"), "ℹ\nx\né\n".as_bytes());
    assert_eq!(bytes("empty_steps"), b"x\n");
    assert_eq!(bytes("word"), b"");
    assert_eq!(bytes("boundary"), b"");
}

#[cfg(target_os = "linux")]
#[test]
fn grep_reads_special_files_that_report_no_size() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("grep -c MemTotal /proc/meminfo > count"), 0);
    assert_eq!(read(&directory, "count"), "1\n");
}

#[test]
fn a_file_that_is_also_the_output_cannot_feed_itself() {
    let directory = crate::test_support::Directory::new();
    fs::write(directory.path().join("both"), "hit\n").unwrap();
    assert_eq!(bounded_script("grep hit both >> both", &directory), 0);
    assert_eq!(read(&directory, "both"), "hit\nhit\n");
}

#[test]
fn file_listings_stop_at_the_first_selected_line() {
    let directory = crate::test_support::Directory::new();
    let script = "while true; do echo hit; done | grep -l hit > listed; \
        while true; do echo hit; done | grep -L hit > unlisted; echo after > marker";
    assert_eq!(bounded_script(script, &directory), 0);
    assert_eq!(read(&directory, "listed"), "(standard input)\n");
    assert_eq!(read(&directory, "unlisted"), "");
    assert_eq!(read(&directory, "marker"), "after\n");
    assert_eq!(over("hit\n", "-lc hit"), (0, "in\n".to_owned()));
    assert_eq!(over("other\n", "-Lc hit"), (1, "in\n".to_owned()));
}

#[test]
fn file_filters_apply_to_operands_and_the_last_match_wins() {
    for (args, status, expected) in [
        (
            "hit tree/a.rs tree/b.txt --include='*.rs'",
            0,
            "tree/a.rs:hit\n",
        ),
        (
            "hit tree/a.rs tree/b.txt --exclude='*.rs'",
            0,
            "tree/b.txt:hit\n",
        ),
        (
            "hit tree/a.rs tree/b.txt --exclude='tree/a.rs'",
            0,
            "tree/b.txt:hit\n",
        ),
        (
            "hit tree/a.rs tree/b.txt --exclude='a.rs'",
            0,
            "tree/b.txt:hit\n",
        ),
        ("hit tree/a.rs --exclude='*'", 1, ""),
        ("hit missing --exclude='*'", 2, ""),
        (
            "-r hit tree/top --exclude='top/a.rs'",
            0,
            "tree/top/a.rs:hit\n",
        ),
        ("-r hit tree/top --include='*.rs' --exclude='a*'", 1, ""),
        (
            "-r hit tree/top --exclude='a*' --include='*.rs'",
            0,
            "tree/top/a.rs:hit\n",
        ),
        ("-r hit tree/top --exclude='a*'", 1, ""),
        ("-r hit tree/top --include='*.none'", 1, ""),
    ] {
        let (directory, mut shell) = shell();
        fs::create_dir_all(directory.path().join("tree/top")).unwrap();
        for name in ["tree/a.rs", "tree/b.txt", "tree/top/a.rs"] {
            fs::write(directory.path().join(name), "hit\n").unwrap();
        }
        let found = shell.run(&format!("grep {args} > out 2> err"));
        assert_eq!(
            (found, read(&directory, "out").as_str()),
            (status, expected),
            "grep {args}"
        );
    }
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("echo hit | grep hit --exclude='*' - > stdin"), 0);
    assert_eq!(read(&directory, "stdin"), "hit\n");
    assert_eq!(shell.run("echo hit | grep hit --exclude='*' > implicit"), 0);
    assert_eq!(read(&directory, "implicit"), "hit\n");
}

#[test]
fn excluded_directories_ignore_trailing_slashes_and_cover_operands() {
    let (directory, mut shell) = shell();
    fs::create_dir_all(directory.path().join("tree/build")).unwrap();
    fs::create_dir_all(directory.path().join("tree/src")).unwrap();
    fs::write(directory.path().join("tree/build/d"), "hit\n").unwrap();
    fs::write(directory.path().join("tree/src/c"), "hit\n").unwrap();
    for (args, status, expected) in [
        ("-r hit tree --exclude-dir=build/", 0, "tree/src/c:hit\n"),
        ("-r hit tree --exclude-dir=build", 0, "tree/src/c:hit\n"),
        (
            "-r hit tree --exclude-dir=tree/build",
            0,
            "tree/build/d:hit\ntree/src/c:hit\n",
        ),
        ("-r hit tree/build --exclude-dir=build", 1, ""),
        ("-r hit tree/build/ --exclude-dir=build", 1, ""),
        ("hit tree/build --exclude-dir=build", 1, ""),
    ] {
        let found = shell.run(&format!("grep {args} > out 2> err"));
        assert_eq!(
            (found, read(&directory, "out").as_str()),
            (status, expected),
            "grep {args}"
        );
    }
}

#[test]
fn context_groups_are_separated_across_files_and_for_zero_lines() {
    let (directory, mut shell) = shell();
    fs::write(directory.path().join("one"), "hit\nctx\nz\n").unwrap();
    fs::write(directory.path().join("two"), "hit\nctx\nz\n").unwrap();
    assert_eq!(shell.run("grep -A1 hit one two > across"), 0);
    assert_eq!(
        read(&directory, "across"),
        "one:hit\none-ctx\n--\ntwo:hit\ntwo-ctx\n"
    );
    assert_eq!(
        over("a\nb\nc\nd\na\n", "-A0 a"),
        (0, "a\n--\na\n".to_owned())
    );
    assert_eq!(over("a\nb\na\n", "-B0 a"), (0, "a\n--\na\n".to_owned()));
    assert_eq!(over("a\na\n", "-A0 a"), (0, "a\na\n".to_owned()));
    assert_eq!(over("a\nb\na\n", "-C0 a"), (0, "a\n--\na\n".to_owned()));
    assert_eq!(over("a\nb\na\n", "a"), (0, "a\na\n".to_owned()));
    assert_eq!(
        over("a\nb\na\nb\n", "-m1 -A5 a"),
        (0, "a\nb\na\nb\n".to_owned())
    );
}

#[test]
fn only_matching_prints_no_context_lines() {
    assert_eq!(
        over("a\nb\nc\na\n", "-o -A1 a"),
        (0, "a\n--\na\n".to_owned())
    );
    assert_eq!(
        over("a\nb\nc\na\n", "-o -B1 a"),
        (0, "a\n--\na\n".to_owned())
    );
    assert_eq!(over("a\nb\na\n", "-o -C1 a"), (0, "a\na\n".to_owned()));
}

#[test]
fn grep_extended_only_matching_uses_leftmost_longest() {
    assert_eq!(grep("grep -Eo 'ok|ok 1' log"), (0, "ok 1\nok\n".into()));
}

#[test]
fn grep_refuses_perl_patterns_it_cannot_interpret() {
    let (status, output) = grep("grep -P 'ok(?= 1)' log");
    assert_eq!(status, 2);
    assert!(output.is_empty());
}
