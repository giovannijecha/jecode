use super::tests::shell;
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
