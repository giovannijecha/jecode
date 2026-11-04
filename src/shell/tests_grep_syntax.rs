//! Pattern syntax. Each expected result was checked against GNU grep 3.12.

use super::tests::shell;
use super::*;

/// Runs `grep ARGS in` with `input` in the file `in`.
pub(super) fn over(input: &str, args: &str) -> (i32, String) {
    let (directory, mut shell) = shell();
    fs::write(directory.path().join("in"), input).unwrap();
    let status = shell.run(&format!("grep {args} in > out 2> err"));
    let output = fs::read_to_string(directory.path().join("out")).unwrap();
    (status, output)
}

fn selects(cases: &[(&str, &str, &str)]) {
    for (input, args, expected) in cases {
        assert_eq!(
            over(input, args),
            (0, (*expected).to_owned()),
            "grep {args} over {input:?}"
        );
    }
}

fn selects_nothing(cases: &[(&str, &str)]) {
    for (input, args) in cases {
        assert_eq!(
            over(input, args),
            (1, String::new()),
            "grep {args} over {input:?}"
        );
    }
}

fn refuses(cases: &[&str]) {
    for args in cases {
        assert_eq!(over("a\n", args), (2, String::new()), "grep {args}");
    }
}

#[test]
fn basic_anchors_and_star_depend_on_their_position() {
    selects(&[
        ("x $HOME y\nz\n", "'$HOME'", "x $HOME y\n"),
        ("price: $5\n", "'price: $5'", "price: $5\n"),
        ("x^2\n", "'x^2'", "x^2\n"),
        ("abc\n*x\n", "'^*'", "*x\n"),
        ("*a\nb\n", "'*a'", "*a\n"),
        ("*a\na\n", "'\\(*a\\)'", "*a\n"),
        ("*a\na\n", "'q\\|*a'", "*a\n"),
        ("ab\nxb\n", "'\\(^a\\)b'", "ab\n"),
        ("ab\nxb\n", "'q\\|^x'", "xb\n"),
        ("ab$\nab\n", "'ab$\\|q'", "ab\n"),
        ("ab\nab$\n", "'\\(ab$\\)'", "ab\n"),
        ("^a\na\n", "'^^a'", "^a\n"),
        ("a$\na\n", "'a$$'", "a$\n"),
        ("a$$$\n", "'a$*$'", "a$$$\n"),
        ("a*\nb\n", "'a\\*'", "a*\n"),
        ("+a\na\n", "'\\+a'", "+a\n"),
        ("a\nxb\n", "-e '^a' -e 'b$'", "a\nxb\n"),
    ]);
    selects_nothing(&[("x^2\n", "-E 'x^2'"), ("a$b\n", "-E 'a$b'")]);
}

#[test]
fn word_matches_only_need_no_word_character_beside_them() {
    selects(&[
        ("fn main() {\n", "-wF 'main()'", "fn main() {\n"),
        ("foo-bar\n", "-w foo", "foo-bar\n"),
        ("xfoo foo\n", "-wo foo", "foo\n"),
        ("foofoo foo\n", "-wo foo", "foo\n"),
        ("foobar foo\n", "-wo 'foo.*'", "foobar foo\n"),
        ("abc abcd\n", "-woE 'abc|abcd'", "abc\nabcd\n"),
        ("a  b\n", "-w ''", "a  b\n"),
        ("\n", "-w ''", "\n"),
    ]);
    selects_nothing(&[
        ("foo_bar\n", "-w foo"),
        ("a b\n", "-w ''"),
        ("a b\n", "-w 'x*'"),
    ]);
}

#[test]
fn unterminated_named_classes_are_errors() {
    refuses(&["'[[:]'", "'[[:a]'", "'[[:]]'", "'[[:foo:]]'", "'[a'"]);
}

#[test]
fn back_references_are_refused_by_name() {
    refuses(&["-E '(a)\\1'", "'\\(a\\)\\1'", "'\\9'"]);
    let (directory, mut shell) = shell();
    assert_eq!(shell.run("grep -E '(a)\\1' missing 2> err"), 2);
    let error = fs::read_to_string(directory.path().join("err")).unwrap();
    assert!(error.contains("back-references"), "{error}");
    selects(&[("\\1\n1\n", "-F '\\1'", "\\1\n")]);
}

#[test]
fn a_leading_extended_repetition_repeats_nothing() {
    selects(&[
        ("abc\n*bc\nq\n", "-oE '*bc'", "bc\nbc\n"),
        ("abc\n+bc\n", "-oE '+bc'", "bc\nbc\n"),
        ("abc\n?bc\n", "-oE '?bc'", "bc\nbc\n"),
        ("abc\n", "-oE 'x|*bc'", "bc\n"),
        ("abc\n", "-oE '(*bc)'", "bc\n"),
        ("*ab\nxab\n", "-E '^*ab'", "*ab\nxab\n"),
    ]);
}

#[test]
fn intervals_follow_gnu_grep() {
    selects(&[
        ("b\na\naa\n", "-oE 'a{,1}'", "a\na\na\n"),
        ("aaa\n", "-oE 'a{,}'", "aaa\n"),
        ("aaa\n", "-oE 'a{2,}'", "aaa\n"),
        ("aaab\n", "-oE 'a{1,2}b'", "aab\n"),
        ("aab\n", "-o 'a\\{2\\}'", "aa\n"),
        ("aab\n", "-o 'a\\{,1\\}b'", "ab\n"),
        ("a{x}\n", "-E 'a{x}'", "a{x}\n"),
        ("a{1\n", "-E 'a{1'", "a{1\n"),
        ("a{1x}\n", "-E 'a{1x}'", "a{1x}\n"),
        ("a{1,x}\n", "-E 'a{1,x}'", "a{1,x}\n"),
        ("a{\n", "-E 'a{'", "a{\n"),
        ("{\n", "-E '{'", "{\n"),
        ("{1}a\na\n", "-E '{1}a'", "{1}a\na\n"),
        ("a{1}\naa\n", "-E 'a\\{1\\}'", "a{1}\n"),
        ("{1}a\na\n", "'\\{1\\}a'", "{1}a\n"),
        ("a{2}\naa\n", "'a{2}'", "a{2}\n"),
    ]);
    selects_nothing(&[("aaa\n", "-oE 'a{2}{2}'")]);
    refuses(&[
        "-E 'a{2,1}'",
        "-E 'a{}'",
        "-E 'a{1,2,3}'",
        "-E 'a{99999}'",
        "-E 'a{99999999999999999999999}'",
        "'a\\{2,1\\}'",
        "'a\\{\\}'",
        "'a\\{x\\}'",
        "'a\\{1'",
        "'a\\{99999\\}'",
    ]);
}

#[test]
fn a_question_mark_after_a_repetition_makes_it_optional() {
    selects(&[
        ("b\n", "-E 'a+?b'", "b\n"),
        ("b\nab\n", "-oE 'a*?b'", "b\nab\n"),
        ("b\naab\n", "-oE 'a{2}?b'", "b\naab\n"),
    ]);
}

#[test]
fn an_unmatched_extended_closing_parenthesis_is_literal() {
    selects(&[
        ("a)\na\n", "-E 'a)'", "a)\n"),
        ("a)\nb\n", "-E 'a)|b'", "a)\nb\n"),
        ("a)\na\n", "-E '(a))'", "a)\n"),
        (")\n", "-E ')'", ")\n"),
    ]);
    refuses(&["-E ')('", "-E '(a'", "'a\\)'"]);
}

#[test]
fn bracket_expressions_follow_posix() {
    selects(&[
        ("a\nb\n", "'[[=a=]]'", "a\n"),
        ("a\nb\n", "'[[.a.]]'", "a\n"),
        ("-\na\n", "'[[.-.]]'", "-\n"),
        (".\na\n", "'[[...]]'", ".\n"),
        ("]\na\n", "'[[.].]]'", "]\n"),
        ("b\nd\n", "'[a-[.c.]]'", "b\n"),
        ("b\nd\n", "'[[.a.]-c]'", "b\n"),
        (".\n\\\nx\n", "'[\\.]'", ".\n\\\n"),
        ("d\n\\\n1\n", "'[\\d]'", "d\n\\\n"),
        ("\\\n", "'[\\]'", "\\\n"),
        ("a\n:\n", "'[:a]'", "a\n:\n"),
        ("a\n:\n", "'[a:]'", "a\n:\n"),
        (":\na\n", "'[::]'", ":\n"),
        (":\nx\n", "'[:a:b]'", ":\n"),
        (":\na\n", "'[:a-z:]'", ":\na\n"),
        (":\na\n", "'[:[:alpha:]:]'", ":\na\n"),
        ("]\n", "'[]]'", "]\n"),
        ("-\nb\na\n", "'[a-]'", "-\na\n"),
        ("-\nb\na\n", "'[-a]'", "-\na\n"),
        ("b\n", "'[b-b]'", "b\n"),
        ("A\n", "-i '[[:lower:]]'", "A\n"),
    ]);
    refuses(&[
        "'[z-a]'",
        "'[a-[:digit:]]'",
        "'[[:digit:]-z]'",
        "'[a-c-e]'",
        "'[[=c=]-d]'",
        "'[a-[=c=]]'",
        "'[[.hyphen.]]'",
        "'[[=ab=]]'",
        "'[[=a]'",
        "'[[.a]'",
        "'[:alpha:]'",
        "'[^:alpha:]'",
        "'[:a:]'",
        "'[]a'",
    ]);
}

#[test]
fn each_pattern_stands_alone() {
    refuses(&["'a\\)'", "'\\(a'", "-E -e 'a(' -e 'b)'"]);
}
