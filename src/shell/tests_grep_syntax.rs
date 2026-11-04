//! Pattern syntax. Each expected result was checked against GNU grep 3.12.

use super::tests::shell;
use super::*;

/// Runs `grep ARGS in` with `input` in the file `in`.
fn over(input: &str, args: &str) -> (i32, String) {
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
fn each_pattern_stands_alone() {
    refuses(&["'a\\)'", "'\\(a'", "-E -e 'a(' -e 'b)'"]);
}
