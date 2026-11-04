use super::tests::shell;
use super::*;
use lexer::{Token, tokenize};

fn read(directory: &crate::test_support::Directory, name: &str) -> String {
    fs::read_to_string(directory.path().join(name)).unwrap()
}

fn ops(script: &str) -> Vec<&'static str> {
    tokenize(script)
        .unwrap()
        .into_iter()
        .filter_map(|spanned| match spanned.token {
            Token::Op(op) => Some(op),
            Token::Word(_) => None,
        })
        .collect()
}

#[test]
fn descriptor_numbers_must_touch_the_redirection() {
    assert_eq!(ops("echo 2>x"), ["2>"]);
    assert_eq!(ops("echo 2 >x"), [">"]);
    assert_eq!(ops("echo '2'>x"), [">"]);
    assert_eq!(ops("echo hi 1>&2"), [">&2"]);
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("seq 1 2 > numbers; echo 1 > one; echo \"2\">quoted; echo hi 2> err 1>&2"),
        0
    );
    assert_eq!(read(&directory, "numbers"), "1\n2\n");
    assert_eq!(read(&directory, "one"), "1\n");
    assert_eq!(read(&directory, "quoted"), "2\n");
    assert_eq!(read(&directory, "err"), "hi\n");
}

#[test]
fn stderr_joins_stdout_in_order_of_redirection() {
    assert_eq!(ops("echo hi 2>&1"), ["2>&1"]);
    assert_eq!(ops("echo hi &> log; echo hi &>> log"), ["&>", ";", "&>>"]);
    assert!(tokenize("echo hi 2>&3").is_err());
    let (directory, mut shell) = shell();
    let script = "(echo b >&2) > both 2>&1; \
        (echo c >&2) 2>&1 | cat > piped; \
        ((echo f >&2) 2>&1 > not_here) > outer; \
        cat missing-entry &> all; echo d &>> all";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "both"), "b\n");
    assert_eq!(read(&directory, "piped"), "c\n");
    assert_eq!(read(&directory, "not_here"), "");
    assert_eq!(read(&directory, "outer"), "f\n");
    let all = read(&directory, "all");
    assert!(
        all.contains("missing-entry") && all.ends_with("d\n"),
        "{all}"
    );
}

#[test]
fn head_and_tail_select_lines_from_files_and_pipes() {
    let (directory, mut shell) = shell();
    let script = "seq 1 20 > numbers; \
        head -n 3 numbers > h3; head -2 numbers > h2; head numbers > h10; \
        tail -n 2 numbers > t2; tail -3 numbers > t3; tail -n +19 numbers > from19; \
        seq 1 5 | head -n2 > piped_head; seq 1 5 | tail -n 1 > piped_tail; \
        printf 'x\\ny' | tail -n 1 > unterminated; \
        head -n 1 numbers numbers > headers; head -n 0 numbers > none";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "h3"), "1\n2\n3\n");
    assert_eq!(read(&directory, "h2"), "1\n2\n");
    assert_eq!(read(&directory, "h10").lines().count(), 10);
    assert_eq!(read(&directory, "t2"), "19\n20\n");
    assert_eq!(read(&directory, "t3"), "18\n19\n20\n");
    assert_eq!(read(&directory, "from19"), "19\n20\n");
    assert_eq!(read(&directory, "piped_head"), "1\n2\n");
    assert_eq!(read(&directory, "piped_tail"), "5\n");
    assert_eq!(read(&directory, "unterminated"), "y");
    assert_eq!(
        read(&directory, "headers"),
        "==> numbers <==\n1\n\n==> numbers <==\n1\n"
    );
    assert_eq!(read(&directory, "none"), "");
    assert_eq!(shell.run("head -c 3 numbers"), 1);
    assert_eq!(shell.run("tail -f numbers"), 1);
}

#[test]
fn missing_programs_name_the_built_ins() {
    let (directory, mut shell) = shell();
    assert_eq!(
        shell.run("jecode-missing-program-fixture 2> err; echo \"$?\" > status"),
        0
    );
    assert_eq!(read(&directory, "status"), "127\n");
    let error = read(&directory, "err");
    assert!(error.contains(builtins::SUMMARY), "{error}");
    for name in builtins::SUMMARY.split(", ") {
        let name = name.split(' ').next().unwrap();
        assert!(builtins::is_builtin(name), "{name}");
    }
    let bash = crate::tools::definitions()
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool.get("name").and_then(crate::json::Value::as_str) == Some("bash"))
        .and_then(|tool| tool.get("description"))
        .and_then(crate::json::Value::as_str)
        .unwrap();
    assert!(bash.contains(builtins::SUMMARY), "{bash}");
}

#[cfg(windows)]
#[test]
fn missing_programs_point_to_a_windows_script_of_the_same_name() {
    let (directory, mut shell) = shell();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("jecode-script-fixture.cmd"), "@echo %1> ran\r\n").unwrap();
    let path = format!("export PATH='{}'", bin.display());
    assert_eq!(
        shell.run(&format!("{path}; jecode-script-fixture 2> err")),
        127
    );
    let error = read(&directory, "err");
    assert!(
        error.contains("run the script as jecode-script-fixture.cmd"),
        "{error}"
    );
    assert_eq!(
        shell.run(&format!("{path}; jecode-script-fixture.cmd argument")),
        0
    );
    assert_eq!(read(&directory, "ran").trim(), "argument");
}

#[test]
fn newlines_after_operators_continue_the_command() {
    let (directory, mut shell) = shell();
    let script = "false &&\necho X > never\necho abc |\n\ntr -d b > joined\ntrue && # note\necho yes > after";
    assert_eq!(shell.run(script), 0);
    assert!(!directory.path().join("never").exists());
    assert_eq!(read(&directory, "joined"), "ac\n");
    assert_eq!(read(&directory, "after"), "yes\n");
}

#[test]
fn compound_commands_join_lists_pipelines_and_redirections() {
    let (directory, mut shell) = shell();
    let script = "true && for f in a b; do echo $f; done > loop
        for f in c d; do echo $f; done | cat > piped
        if true; then echo yes; fi > cond
        false || while false; do :; done && (echo sub) > subshell";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "loop"), "a\nb\n");
    assert_eq!(read(&directory, "piped"), "c\nd\n");
    assert_eq!(read(&directory, "cond"), "yes\n");
    assert_eq!(read(&directory, "subshell"), "sub\n");
}

#[test]
fn nested_loops_and_conditionals_match_their_own_keywords() {
    let (directory, mut shell) = shell();
    let script = "for i in 1 2; do for j in a b; do echo $i$j; done; done > grid
        if true; then if false; then echo no; elif true; then echo inner; else echo no; fi; echo outer; fi > branch
        for ((n=0; n<2; n++)); do until true; do :; done; echo $n; done > counted";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "grid"), "1a\n1b\n2a\n2b\n");
    assert_eq!(read(&directory, "branch"), "inner\nouter\n");
    assert_eq!(read(&directory, "counted"), "0\n1\n");
}

#[test]
fn all_arguments_expand_to_one_word_each() {
    let (directory, mut shell) = shell();
    shell.positional = vec!["x y".into(), "z".into()];
    assert_eq!(
        shell.run("for a in \"$@\"; do echo \"<$a>\"; done > each; echo \"[$@]\" > joined"),
        0
    );
    assert_eq!(read(&directory, "each"), "<x y>\n<z>\n");
    assert_eq!(read(&directory, "joined"), "[x y z]\n");
    shell.positional.clear();
    assert_eq!(shell.run("for a in \"$@\"; do echo $a; done > none"), 0);
    assert_eq!(read(&directory, "none"), "");
}

#[test]
fn background_lists_expand_in_the_child_shell() {
    let (directory, mut shell) = shell();
    let script = "(X=1; echo \"x=$X\" > first) & wait; echo pre > pre; (echo sub > second) & wait";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "first"), "x=1\n");
    assert_eq!(read(&directory, "second"), "sub\n");
    assert_eq!(shell.variables.get("X"), None);
}

#[test]
fn literal_braces_are_words_but_groups_and_expansion_fail() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("echo {} HEAD@{1} '{a,b}' > braces"), 0);
    assert_eq!(read(&directory, "braces"), "{} HEAD@{1} {a,b}\n");
    assert_eq!(shell.run("touch made; echo {a,b}"), 2);
    assert_eq!(shell.run("touch made; { echo grouped; }"), 2);
    assert!(!directory.path().join("made").exists());
}

#[test]
fn unsupported_substitution_syntax_fails_before_prior_commands_run() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("touch made; echo $(echo {a,b})"), 2);
    assert!(!directory.path().join("made").exists());
    assert_eq!(shell.run("touch made; echo \"`pwd`\""), 2);
    assert!(!directory.path().join("made").exists());
    assert_eq!(shell.run("echo '`literal`' > quoted"), 0);
    assert_eq!(read(&directory, "quoted"), "`literal`\n");
}

#[test]
fn escaped_closing_parenthesis_stays_inside_command_substitution() {
    let (directory, mut shell) = shell();
    assert_eq!(shell.run(r#"echo $(printf \)) > result"#), 0);
    assert_eq!(read(&directory, "result"), ")\n");
}

#[test]
fn command_substitution_reads_here_document_bodies_as_text() {
    let (directory, mut shell) = shell();
    let script = "echo \"$(cat <<'EOF'\nSubject\n\nit's a fix (for \"quotes)\nEOF\n)\" > message\n\
        echo \"$(cat <<EOF\nvalue ) $(echo 'inner)')\nEOF\n)\" > expanded\n\
        echo \"$(cat <<EOF\nbody\nEOF)\" > closed_on_delimiter\n\
        echo \"$(cat <<-EOF\n\t' tabbed )\n\tEOF\n)\" > tabs\n\
        echo \"$(cat <<A; cat <<B\n'a\nA\n)b\nB\n)\" > two\n\
        echo $(tr -d o <<< \"foo)\") > here_string\n\
        echo \"$(echo keep # don't close )\n)\" > comment\n\
        echo \"$(echo not#comment)\" > hash\n";
    assert_eq!(shell.run(script), 0);
    assert_eq!(
        read(&directory, "message"),
        "Subject\n\nit's a fix (for \"quotes)\n"
    );
    assert_eq!(read(&directory, "expanded"), "value ) inner)\n");
    assert_eq!(read(&directory, "closed_on_delimiter"), "body\n");
    assert_eq!(read(&directory, "tabs"), "' tabbed )\n");
    assert_eq!(read(&directory, "two"), "'a\n)b\n");
    assert_eq!(read(&directory, "here_string"), "f)\n");
    assert_eq!(read(&directory, "comment"), "keep\n");
    assert_eq!(read(&directory, "hash"), "not#comment\n");
    assert!(tokenize("echo \"$(cat <<EOF\nnever closed)\"").is_err());
    assert!(tokenize("echo $(cat <<)").is_err());
}

#[test]
fn set_e_stops_only_on_unhandled_failures() {
    let (directory, mut shell) = shell();
    let script = "set -e; false || true; false && true; if false; then :; fi; echo ok > ok; false; echo no > no";
    assert_eq!(shell.run(script), 1);
    assert!(directory.path().join("ok").exists());
    assert!(!directory.path().join("no").exists());
}

#[test]
fn long_scripts_run_without_deep_recursion() {
    let (directory, mut shell) = shell();
    let mut script = "x=0\n".repeat(5000);
    script.push_str("if true; then\n");
    script.push_str(&"echo line\n".repeat(5000));
    script.push_str("fi > lines\n");
    assert_eq!(shell.run(&script), 0);
    assert_eq!(read(&directory, "lines").len(), 5000 * "line\n".len());
}

#[test]
fn here_documents_feed_standard_input() {
    let (directory, mut shell) = shell();
    let script = "NAME=world\n\
        cat > literal <<'EOF'\nconst q = 'it\'s' + `${x}`; $NAME\nEOF\n\
        cat > expanded <<EOF\nhello $NAME \\$HOME \"quoted\"\nEOF\n\
        cat <<-END > tabs\n\tindented\n\tEND\n\
        cat >> literal << \"STOP\"\n2 > 1 && echo no\nSTOP\n\
        tr -d o <<< \"foo\" > here_string\n\
        echo after > after";
    assert_eq!(shell.run(script), 0);
    assert_eq!(
        read(&directory, "literal"),
        "const q = 'it\'s' + `${x}`; $NAME\n2 > 1 && echo no\n"
    );
    assert_eq!(
        read(&directory, "expanded"),
        "hello world $HOME \"quoted\"\n"
    );
    assert_eq!(read(&directory, "tabs"), "indented\n");
    assert_eq!(read(&directory, "here_string"), "f\n");
    assert_eq!(read(&directory, "after"), "after\n");
    assert!(!directory.path().join("no").exists());
    for broken in [
        "cat <<EOF",
        "cat <<EOF\nbody",
        "cat <<\nx",
        "cat <<EOF\n`date`\nEOF",
    ] {
        assert!(tokenize(broken).is_err(), "{broken}");
    }
}

#[test]
fn background_lists_keep_here_document_bodies() {
    let (directory, mut shell) = shell();
    let script = "cat <<FIRST | cat > piped &\nfrom pipeline\nFIRST\n\
        (cat <<SECOND) > grouped &\nfrom subshell\nSECOND\n\
        (cat <<INSIDE\ninside subshell\nINSIDE\n) > inside &\n\
        cat <<LEFT | cat <<RIGHT > multiple &\nignored\nLEFT\nsecond body\nRIGHT\n\
        wait";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "piped"), "from pipeline\n");
    assert_eq!(read(&directory, "grouped"), "from subshell\n");
    assert_eq!(read(&directory, "inside"), "inside subshell\n");
    assert_eq!(read(&directory, "multiple"), "second body\n");
}

#[test]
fn unquoted_patterns_expand_to_sorted_paths() {
    let (directory, mut shell) = shell();
    let script = "mkdir -p src/nested test; touch src/b.js src/a.js src/c.rs src/.hidden.js src/nested/d.js test/x.test.js; \
        echo src/*.js > star; echo src/?.rs > question; echo src/[ab].js > class; \
        echo src/[!a].js > negated; echo src/*/*.js > nested; echo 'src/*.js' \"src/*.js\" > quoted; \
        echo src/*.none > unmatched; echo src/.*.js > dotted; X=src/*.js; echo \"$X\" > assigned; \
        for f in test/*.js; do echo \"$f\" > looped; done; \
        if [ -d src ]; then echo yes > bracket; fi; cat src/*.js > joined";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "star"), "src/a.js src/b.js\n");
    assert_eq!(read(&directory, "question"), "src/c.rs\n");
    assert_eq!(read(&directory, "class"), "src/a.js src/b.js\n");
    assert_eq!(read(&directory, "negated"), "src/b.js\n");
    assert_eq!(read(&directory, "nested"), "src/nested/d.js\n");
    assert_eq!(read(&directory, "quoted"), "src/*.js src/*.js\n");
    assert_eq!(read(&directory, "unmatched"), "src/*.none\n");
    assert_eq!(read(&directory, "dotted"), "src/.hidden.js\n");
    assert_eq!(read(&directory, "assigned"), "src/*.js\n");
    assert_eq!(read(&directory, "looped"), "test/x.test.js\n");
    assert_eq!(read(&directory, "bracket"), "yes\n");
    assert_eq!(read(&directory, "joined"), "");
}
