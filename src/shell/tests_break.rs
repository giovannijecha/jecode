use super::tests::shell;
use super::*;

fn read(directory: &crate::test_support::Directory, name: &str) -> String {
    fs::read_to_string(directory.path().join(name)).unwrap()
}

const REPORTED: &str = "for ((i=0; i<3; i++)); do
  echo before-$i
  break
  echo after-$i
done
echo done";

#[test]
fn the_reported_counted_loop_breaks_once_through_the_bash_tool() {
    let directory = crate::test_support::Directory::new();
    let tools = crate::tools::Tools::new(directory.path()).unwrap();
    let result = tools.execute(
        "bash",
        &crate::json::Value::object([("command", crate::json::Value::string(REPORTED))]).encode(),
    );
    assert_eq!(
        result.get("stdout").and_then(crate::json::Value::as_str),
        Some("before-0\ndone\n"),
        "{result:?}"
    );
    assert_eq!(
        result.get("stderr").and_then(crate::json::Value::as_str),
        Some(""),
        "{result:?}"
    );
    assert_eq!(
        result.get("exit_code"),
        Some(&crate::json::Value::number(0))
    );
}

#[test]
fn every_loop_form_ends_at_its_break() {
    let (directory, mut shell) = shell();
    let script = "
for ((i=0; i<3; i++)); do echo before-$i; break; echo after-$i; done > counted
for x in a b c; do if test $x = b; then break; fi; echo $x; done > words
while true; do echo w; break; echo never; done > while
until false; do echo u; break; echo never; done > until
echo end > end";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "counted"), "before-0\n");
    assert_eq!(read(&directory, "words"), "a\n");
    assert_eq!(read(&directory, "while"), "w\n");
    assert_eq!(read(&directory, "until"), "u\n");
}

#[test]
fn break_leaves_only_its_own_loop_unless_given_a_count() {
    let (directory, mut shell) = shell();
    let script = "
for a in 1 2; do for b in x y; do echo $a$b; break; done; echo after-$a; done > one
for a in 1 2; do for b in x y; do echo $a$b; break 2; done; echo never; done > two
echo next >> two
for a in 1 2; do while true; do echo $a; break 9; done; echo never; done > clamped
echo next >> clamped";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "one"), "1x\nafter-1\n2x\nafter-2\n");
    assert_eq!(read(&directory, "two"), "1x\nnext\n");
    assert_eq!(read(&directory, "clamped"), "1\nnext\n");
}

#[test]
fn a_decimal_count_larger_than_usize_leaves_the_outermost_loop() {
    let (directory, mut shell) = shell();
    let script = "
for a in 1 2; do for b in x y; do echo $a$b; break 184467440737095516160000000000; done; echo never; done > huge
echo next >> huge
for a in 1 2; do echo $a; break 00000000000000000000000000000001; done > leading";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "huge"), "1x\nnext\n");
    assert_eq!(read(&directory, "leading"), "1\n");
}

#[test]
fn break_in_conditions_and_chains_stops_the_rest_of_the_list() {
    let (directory, mut shell) = shell();
    let script = "
for x in 1 2; do true && break || echo no; echo never; done > chain
echo end >> chain
while break; do echo never; done; echo $? > condition
for x in 1 2; do if break; then echo never; fi; echo never; done > in-if
echo end >> in-if";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "chain"), "end\n");
    assert_eq!(read(&directory, "condition"), "0\n");
    assert_eq!(read(&directory, "in-if"), "end\n");
}

#[test]
fn invalid_break_reports_an_error_and_leaves_no_loop() {
    let (directory, mut shell) = shell();
    let script = "
break 2> outside-err; echo $? > outside
for x in 1 2; do break 0 2>> count-err; echo $x; done > zero
for x in 1; do break x 2>> count-err; break -1 2>> count-err; echo $x; done > word
for x in 1; do break 1 2 2> many-err; echo $x; done > many";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "outside"), "1\n");
    assert!(read(&directory, "outside-err").contains("only meaningful inside"));
    assert_eq!(read(&directory, "zero"), "1\n2\n");
    assert_eq!(read(&directory, "word"), "1\n");
    let errors = read(&directory, "count-err");
    for argument in ["0", "x", "-1"] {
        assert!(
            errors.contains(&format!(
                "break: {argument}: expected a positive loop count"
            )),
            "{errors}"
        );
    }
    assert_eq!(read(&directory, "many"), "1\n");
    assert!(read(&directory, "many-err").contains("at most one loop count"));
}

#[test]
fn subshells_and_pipeline_stages_cannot_leave_the_parent_loop() {
    let (directory, mut shell) = shell();
    let script = "
for x in 1 2; do (break) 2>> subshell-err; echo $x; done > subshell
for x in 1 2; do echo hi | break 2>> pipe-err; echo $x; done > pipe
for x in 1 2; do (for y in a b; do echo $y; break; done); echo $x; done > inner";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "subshell"), "1\n2\n");
    assert!(read(&directory, "subshell-err").contains("only meaningful inside"));
    assert_eq!(read(&directory, "pipe"), "1\n2\n");
    assert!(read(&directory, "pipe-err").contains("only meaningful inside"));
    assert_eq!(read(&directory, "inner"), "a\n1\na\n2\n");
}

#[test]
fn break_keeps_ordinary_exit_status_semantics() {
    let (directory, mut shell) = shell();
    let script = "
for x in 1; do false; break; done; echo $? > broken
set -e
for x in 1 2; do break; done
echo kept > kept";
    assert_eq!(shell.run(script), 0);
    assert_eq!(read(&directory, "broken"), "0\n");
    assert_eq!(read(&directory, "kept"), "kept\n");
    // exit still ends the whole script from inside a loop.
    let (directory, mut shell) = super::tests::shell();
    assert_eq!(
        shell.run("for x in 1 2; do exit 4; done; echo never > never"),
        4
    );
    assert!(!directory.path().join("never").exists());
}
