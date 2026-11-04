use super::*;

fn view(result: &str) -> Value {
    let message = Value::object([
        ("role", Value::string("tool")),
        ("tool_call_id", Value::string("one")),
        ("content", Value::string(result)),
    ]);
    let projected = compact(&message);
    assert_eq!(projected.get("role"), message.get("role"));
    assert_eq!(projected.get("tool_call_id"), message.get("tool_call_id"));
    json::parse(projected.get("content").and_then(Value::as_str).unwrap()).unwrap()
}

#[test]
fn a_finished_command_keeps_what_it_said() {
    let result = view(
        r#"{"cancelled":false,"checkpoint":{"saved_paths":0,"status":"none"},"exit_code":0,"output_limit_bytes":16384,"stderr":"","stderr_bytes":0,"stderr_file":"output:a:stderr","stderr_truncated":false,"stdout":"ok\n","stdout_bytes":3,"stdout_file":"output:a:stdout","stdout_truncated":false,"timed_out":false,"truncated":false}"#,
    );
    assert_eq!(result.encode(), r#"{"exit_code":0,"stdout":"ok\n"}"#);
}

#[test]
fn partial_output_keeps_its_saved_files_and_sizes() {
    let result = view(
        r#"{"exit_code":1,"stderr":"","stderr_bytes":0,"stderr_file":"output:a:stderr","stdout":"head","stdout_bytes":90000,"stdout_file":"output:a:stdout","stdout_truncated":true,"truncated":true}"#,
    );
    for key in [
        "stdout_file",
        "stdout_bytes",
        "stdout_truncated",
        "truncated",
    ] {
        assert!(
            result.get(key).is_some(),
            "{key} tells the model where to look"
        );
    }
    assert_eq!(result.get("exit_code"), Some(&Value::number(1)));
}

#[test]
fn a_running_process_keeps_its_output_files() {
    let result = view(r#"{"process_id":"p1","status":"running","stdout_file":"output:a:stdout"}"#);
    assert!(result.get("stdout_file").is_some());
}

#[test]
fn values_that_change_the_meaning_stay() {
    let result = view(
        r#"{"complete":false,"errors":[{"path":"a","error":"denied"}],"errors_count":1,"errors_truncated":true,"exit_code":0,"offset":40,"byte_offset":12,"found":false,"text":""}"#,
    );
    for key in [
        "complete",
        "errors",
        "errors_count",
        "errors_truncated",
        "exit_code",
        "offset",
        "byte_offset",
        "found",
        "text",
    ] {
        assert!(result.get(key).is_some(), "{key} must stay");
    }
}

#[test]
fn file_pages_keep_only_their_position_when_it_matters() {
    let result = view(
        r#"{"files":[{"byte_offset":0,"bytes_returned":10,"content":"1: a\n","lines_returned":1,"next_byte_offset":null,"next_offset":null,"offset":1,"output_limit_bytes":65536,"partial_line":false,"path":"a.txt","total_lines":1,"truncated":false},{"content":"41: b\n","next_offset":42,"offset":41,"path":"b.txt"}]}"#,
    );
    assert_eq!(
        result.encode(),
        r#"{"files":[{"content":"1: a\n","path":"a.txt","total_lines":1},{"content":"41: b\n","next_offset":42,"offset":41,"path":"b.txt"}]}"#
    );
}

#[test]
fn a_saved_checkpoint_keeps_its_reference() {
    let result = view(
        r#"{"bytes_written":3,"checkpoint":{"expires_at":179,"ref":"rc-1","saved_paths":1,"status":"saved"},"path":"a"}"#,
    );
    assert_eq!(
        result.get("checkpoint").unwrap().encode(),
        r#"{"ref":"rc-1"}"#
    );
    let failed = view(r#"{"checkpoint":{"reason":"disk full","status":"failed"}}"#);
    assert_eq!(
        failed.get("checkpoint").unwrap().get("reason"),
        Some(&Value::string("disk full"))
    );
}

#[test]
fn search_matches_read_like_grep_and_merge_overlapping_context() {
    let result = view(
        r#"{"complete":true,"errors":[],"errors_count":0,"errors_truncated":false,"files_searched":2,"matches":[
            {"after":[{"line":3,"text":"second","text_truncated":false}],"before":[{"line":1,"text":"zero","text_truncated":false}],"byte_offset":9,"line":2,"path":"a.js","text":"first","text_truncated":false},
            {"after":[],"before":[{"line":2,"text":"first","text_truncated":false}],"byte_offset":20,"line":3,"path":"a.js","text":"second","text_truncated":false},
            {"after":[],"before":[],"byte_offset":90,"line":9,"path":"a.js","text":"far","text_truncated":false},
            {"after":[],"before":[],"byte_offset":5,"line":1,"path":"b.js","text":"other","text_truncated":false}
        ],"matches_returned":4,"next_offset":null,"offset":1,"output_limit_bytes":65536,"path":".","skipped":{"binary":0,"hidden":3,"ignored":0},"truncated":false}"#,
    );
    assert_eq!(
        result.get("matches").and_then(Value::as_str),
        Some("a.js\n1-zero\n2:first\n3:second\n--\n9:far\nb.js\n1:other\n")
    );
    assert_eq!(result.get("skipped").unwrap().encode(), r#"{"hidden":3}"#);
    assert!(result.get("errors").is_none());
    assert_eq!(result.get("matches_returned"), Some(&Value::number(4)));
}

#[test]
fn a_cut_search_line_says_where_it_continues() {
    let result = view(
        r#"{"matches":[{"after":[],"before":[],"byte_offset":7,"line":1,"path":"a","text":"long","text_truncated":true}]}"#,
    );
    assert_eq!(
        result.get("matches").and_then(Value::as_str),
        Some("a\n1:long [line cut; read from byte_offset 7]\n")
    );
}

#[test]
fn step_results_are_trimmed_like_their_tools() {
    let result = view(
        r#"{"steps":[{"result":{"cancelled":false,"exit_code":0,"stderr":"","stdout":"ok"},"tool":"bash"},{"outcome":"not_run","tool":"read"}]}"#,
    );
    assert_eq!(
        result.encode(),
        r#"{"steps":[{"result":{"exit_code":0,"stdout":"ok"},"tool":"bash"},{"outcome":"not_run","tool":"read"}]}"#
    );
}

#[test]
fn other_contents_pass_through_unchanged() {
    for content in ["plain text", "[1,2]", "{broken"] {
        let message = Value::object([
            ("role", Value::string("tool")),
            ("content", Value::string(content)),
        ]);
        assert_eq!(compact(&message), message);
    }
}

#[test]
fn the_view_is_stable_across_requests() {
    let result = r#"{"exit_code":0,"stdout":"same","truncated":false}"#;
    assert_eq!(view(result).encode(), view(result).encode());
    let again = view(&view(result).encode());
    assert_eq!(again.encode(), view(result).encode());
}
