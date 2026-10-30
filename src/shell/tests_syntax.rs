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
