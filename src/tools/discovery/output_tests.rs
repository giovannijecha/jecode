//! `search` on saved command output, through the store `read` uses.
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::permissions::Kind;
use crate::redact::Redactor;
use crate::test_support::Directory;
use crate::tools::Tools;

struct Fixture {
    _project: Directory,
    home: Directory,
    tools: Tools,
    session: String,
}

fn fixture() -> Fixture {
    let project = Directory::new();
    let home = Directory::new();
    let mut tools = Tools::new(project.path()).unwrap();
    let session = crate::sessions::identifier();
    tools
        .configure_session_output(home.path().join("sessions"), &session, Redactor::empty())
        .unwrap();
    Fixture {
        _project: project,
        home,
        tools,
        session,
    }
}

fn saved(tools: &Tools, stdout: &[u8], stderr: &[u8]) -> (String, String) {
    let mut logs = tools.outputs().create().unwrap();
    logs.stdout.write(stdout).unwrap();
    logs.stdout.finish().unwrap();
    logs.stderr.write(stderr).unwrap();
    logs.stderr.finish().unwrap();
    (logs.stdout_ref, logs.stderr_ref)
}

fn search(tools: &Tools, path: &str, query: &str, extra: &[(&str, Value)]) -> Value {
    search_with(tools, path, query, extra, &Cancellation::default())
}

fn search_with(
    tools: &Tools,
    path: &str,
    query: &str,
    extra: &[(&str, Value)],
    cancellation: &Cancellation,
) -> Value {
    let mut arguments = Value::object([
        ("path", Value::string(path)),
        ("query", Value::string(query)),
    ]);
    if let Value::Object(fields) = &mut arguments {
        for (key, value) in extra {
            fields.insert((*key).into(), value.clone());
        }
    }
    tools.execute_with_cancel("search", &arguments.encode(), cancellation)
}

fn matches(result: &Value) -> &[Value] {
    result
        .get("matches")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no matches in {result:?}"))
}

fn error(result: &Value) -> &str {
    result
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected an error: {result:?}"))
}

fn texts(values: &Value) -> Vec<&str> {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.get("text").and_then(Value::as_str).unwrap())
        .collect()
}

#[test]
fn both_streams_are_searched_by_reference_with_context_and_usable_offsets() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let (stdout, stderr) = saved(
        tools,
        b"compiling a\ncompiling b\nerror: broken b\nnote: see b\ndone\n",
        b"warning: first\nerror[E0308]: mismatched\nhelp: convert\n",
    );
    let result = search(tools, &stdout, "error", &[("context", Value::number(1))]);
    let found = matches(&result);
    assert_eq!(found.len(), 1, "{result:?}");
    let hit = &found[0];
    assert_eq!(
        hit.get("path").and_then(Value::as_str),
        Some(stdout.as_str())
    );
    assert_eq!(hit.get("line"), Some(&Value::number(3)));
    assert_eq!(texts(hit.get("before").unwrap()), ["compiling b"]);
    assert_eq!(texts(hit.get("after").unwrap()), ["note: see b"]);
    assert_eq!(
        result.get("path").and_then(Value::as_str),
        Some(stdout.as_str())
    );
    assert_eq!(result.get("complete"), Some(&Value::Bool(true)));
    assert_eq!(result.get("files_searched"), Some(&Value::number(1)));
    // The offset recovers the match through read on the same reference.
    let offset = hit.get("byte_offset").and_then(Value::as_usize).unwrap();
    let read = tools.execute(
        "read",
        &Value::object([
            ("path", Value::string(&stdout)),
            ("byte_offset", Value::number(offset)),
        ])
        .encode(),
    );
    let content = read.get("content").and_then(Value::as_str).unwrap();
    assert!(content.starts_with("error: broken b"), "{read:?}");

    let result = search(
        tools,
        &stderr,
        "error[E0308]",
        &[("context", Value::number(0))],
    );
    let hit = &matches(&result)[0];
    assert_eq!(
        hit.get("path").and_then(Value::as_str),
        Some(stderr.as_str())
    );
    assert_eq!(hit.get("line"), Some(&Value::number(2)));
    assert_eq!(hit.get("before"), Some(&Value::Array(Vec::new())));
    // Results name logical references, never the private storage path.
    let home = fixture
        .home
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(!result.encode().contains(&home), "{result:?}");
}

#[test]
fn pagination_and_case_follow_ordinary_search() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let text: String = (1..=5).map(|line| format!("Hit {line}\nmiss\n")).collect();
    let (stdout, _) = saved(tools, text.as_bytes(), b"");
    let sensitive = search(tools, &stdout, "hit", &[]);
    assert!(matches(&sensitive).is_empty(), "{sensitive:?}");
    let first = search(
        tools,
        &stdout,
        "hit",
        &[
            ("case_sensitive", Value::Bool(false)),
            ("limit", Value::number(2)),
            ("context", Value::number(0)),
        ],
    );
    assert_eq!(matches(&first).len(), 2);
    assert_eq!(first.get("next_offset"), Some(&Value::number(3)));
    assert_eq!(first.get("complete"), Some(&Value::Bool(false)));
    let last = search(
        tools,
        &stdout,
        "hit",
        &[
            ("case_sensitive", Value::Bool(false)),
            ("offset", Value::number(5)),
            ("context", Value::number(0)),
        ],
    );
    let found = matches(&last);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].get("text").and_then(Value::as_str), Some("Hit 5"));
    assert_eq!(found[0].get("line"), Some(&Value::number(9)));
    assert_eq!(last.get("next_offset"), Some(&Value::Null));
    assert_eq!(last.get("complete"), Some(&Value::Bool(true)));
}

#[test]
fn utf8_offsets_count_bytes_and_invalid_text_is_reported_without_losing_matches() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let line = "café – 漢字 needle ✓";
    let mut bytes = format!("{line}\nsecond needle\n").into_bytes();
    bytes.extend_from_slice(b"\xff\xfe broken needle\n");
    let (stdout, _) = saved(tools, &bytes, b"");
    let result = search(tools, &stdout, "needle", &[("context", Value::number(0))]);
    let found = matches(&result);
    assert_eq!(found.len(), 2, "{result:?}");
    assert_eq!(found[0].get("text").and_then(Value::as_str), Some(line));
    assert_eq!(
        found[0].get("byte_offset"),
        Some(&Value::number(line.find("needle").unwrap()))
    );
    assert_eq!(
        found[1].get("byte_offset"),
        Some(&Value::number(line.len() + 1 + "second ".len()))
    );
    assert_eq!(result.get("complete"), Some(&Value::Bool(false)));
    let errors = result.get("errors").and_then(Value::as_array).unwrap();
    assert_eq!(
        errors[0].get("path").and_then(Value::as_str),
        Some(stdout.as_str())
    );
    assert!(
        errors[0]
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("not UTF-8")
    );
}

#[test]
fn an_oversized_match_stays_within_the_output_limit() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let long = format!("{}needle{}\n", "a".repeat(200_000), "b".repeat(200_000));
    let (stdout, _) = saved(tools, long.as_bytes(), b"");
    let result = search(tools, &stdout, "needle", &[]);
    assert!(result.encode().len() <= crate::tools::OUTPUT_LIMIT);
    let hit = &matches(&result)[0];
    assert_eq!(hit.get("text_truncated"), Some(&Value::Bool(true)));
    assert_eq!(hit.get("byte_offset"), Some(&Value::number(200_000)));
    assert!(
        hit.get("text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("needle")
    );
}

#[test]
fn cancellation_stops_before_reading() {
    let fixture = fixture();
    let (stdout, _) = saved(&fixture.tools, b"needle\n", b"");
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let result = search_with(&fixture.tools, &stdout, "needle", &[], &cancellation);
    assert!(error(&result).contains("cancelled"), "{result:?}");
}

#[test]
fn malformed_unavailable_and_foreign_references_are_refused() {
    let fixture = fixture();
    let tools = &fixture.tools;
    assert!(error(&search(tools, "output:bad", "x", &[])).contains("Invalid output reference"));
    assert!(
        error(&search(tools, "output:../x:1:stdout", "x", &[]))
            .contains("Invalid output reference")
    );
    let missing = format!(
        "output:{}:{}:stdout",
        fixture.session,
        crate::sessions::identifier()
    );
    let unavailable = search(tools, &missing, "x", &[]);
    assert!(
        error(&unavailable).contains("not saved")
            || error(&unavailable).contains("no longer saved")
    );
    let unknown = format!("output:{}:1-1-1:stdout", crate::sessions::identifier());
    assert!(error(&search(tools, &unknown, "x", &[])).contains("not saved for this conversation"));

    // Another folder's session in the same home is not this conversation's.
    let other_project = Directory::new();
    let mut other = Tools::new(other_project.path()).unwrap();
    let other_session = crate::sessions::identifier();
    other
        .configure_session_output(
            fixture.home.path().join("sessions"),
            &other_session,
            Redactor::empty(),
        )
        .unwrap();
    let (foreign, _) = saved(&other, b"secret needle\n", b"");
    let refused = search(tools, &foreign, "needle", &[]);
    assert!(
        error(&refused).contains("belongs to another folder"),
        "{refused:?}"
    );

    // A stream replaced by a directory is not a saved regular file.
    let (stdout, _) = saved(tools, b"needle\n", b"");
    let path = tools.outputs().resolve(&stdout).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(error(&search(tools, &stdout, "needle", &[])).contains("regular file"));
}

#[test]
fn a_linked_stream_entry_is_refused_rather_than_followed() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let (stdout, _) = saved(tools, b"needle\n", b"");
    let target = tools.outputs().resolve(&stdout).unwrap();
    let link = target.with_file_name("9-9-9.stdout");
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(&target, &link).is_err() {
        return; // Windows requires Developer Mode or the symlink privilege.
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let linked = format!("output:{}:9-9-9:stdout", fixture.session);
    let result = search(tools, &linked, "needle", &[]);
    assert!(error(&result).contains("link or junction"), "{result:?}");
}

#[test]
fn a_stream_replaced_by_a_directory_after_resolution_is_not_walked() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let (stdout, _) = saved(tools, b"original\n", b"");
    let resolved = tools.outputs().resolve(&stdout).unwrap();
    std::fs::remove_file(&resolved).unwrap();
    std::fs::create_dir(&resolved).unwrap();
    std::fs::write(resolved.join("other.txt"), b"needle\n").unwrap();

    // Model the change after Store::resolve but before the search walk.
    let result = super::search_output(
        &stdout,
        &resolved,
        &Value::object([
            ("path", Value::string(&stdout)),
            ("query", Value::string("needle")),
        ]),
        &Cancellation::default(),
    )
    .unwrap();
    assert!(matches(&result).is_empty(), "{result:?}");
    assert_eq!(result.get("complete"), Some(&Value::Bool(false)));
    assert_eq!(result.get("files_searched"), Some(&Value::number(0)));
    let errors = result.get("errors").and_then(Value::as_array).unwrap();
    assert_eq!(errors[0].get("path"), Some(&Value::string(&stdout)));
    assert!(
        errors[0]
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("no longer names a regular file")
    );
    assert!(
        !result
            .encode()
            .contains(&resolved.to_string_lossy().to_string())
    );
}

#[cfg(unix)]
#[test]
fn a_stream_replaced_by_a_link_after_resolution_is_an_error() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let (stdout, _) = saved(tools, b"original\n", b"");
    let resolved = tools.outputs().resolve(&stdout).unwrap();
    let outside = fixture.home.path().join("other.txt");
    std::fs::write(&outside, b"needle\n").unwrap();
    std::fs::remove_file(&resolved).unwrap();
    std::os::unix::fs::symlink(&outside, &resolved).unwrap();

    let result = super::search_output(
        &stdout,
        &resolved,
        &Value::object([
            ("path", Value::string(&stdout)),
            ("query", Value::string("needle")),
        ]),
        &Cancellation::default(),
    )
    .unwrap();
    assert!(matches(&result).is_empty(), "{result:?}");
    assert_eq!(result.get("complete"), Some(&Value::Bool(false)));
    let errors = result.get("errors").and_then(Value::as_array).unwrap();
    assert_eq!(errors[0].get("path"), Some(&Value::string(&stdout)));
    assert!(
        !result
            .encode()
            .contains(&outside.to_string_lossy().to_string())
    );
}

#[test]
fn filesystem_options_are_defined_for_one_saved_stream() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let (stdout, _) = saved(tools, b"needle\n", b"");
    let included = search(
        tools,
        &stdout,
        "needle",
        &[
            ("include_hidden", Value::Bool(true)),
            ("include_ignored", Value::Bool(false)),
        ],
    );
    assert_eq!(matches(&included).len(), 1, "{included:?}");
    let pattern = search(tools, &stdout, "needle", &[("pattern", Value::string("*"))]);
    assert!(error(&pattern).contains("pattern selects project paths"));
    let invalid = search(
        tools,
        &stdout,
        "needle",
        &[("include_hidden", Value::string("yes"))],
    );
    assert!(error(&invalid).contains("include_hidden must be a boolean"));
    let list = tools.execute(
        "list",
        &Value::object([("path", Value::string(&stdout))]).encode(),
    );
    assert!(error(&list).contains("use read or search"), "{list:?}");
}

#[test]
fn saved_output_stays_read_only_and_searchable_at_every_level() {
    let fixture = fixture();
    let tools = &fixture.tools;
    let (stdout, _) = saved(tools, b"needle\n", b"");
    for (name, arguments) in [
        (
            "write",
            Value::object([
                ("path", Value::string(&stdout)),
                ("content", Value::string("x")),
            ]),
        ),
        (
            "edit",
            Value::object([
                ("path", Value::string(&stdout)),
                ("old_text", Value::string("needle")),
                ("new_text", Value::string("x")),
            ]),
        ),
    ] {
        let result = tools.execute(name, &arguments.encode());
        assert!(error(&result).contains("read-only"), "{name}: {result:?}");
    }
    assert_eq!(
        std::fs::read(tools.outputs().resolve(&stdout).unwrap()).unwrap(),
        b"needle\n"
    );
    let access = tools
        .access(
            "search",
            &Value::object([
                ("path", Value::string(&stdout)),
                ("query", Value::string("x")),
            ])
            .encode(),
        )
        .unwrap();
    assert_eq!(access.kind, Kind::Reads);
    assert!(
        !access.outside,
        "a saved output is the conversation's own data"
    );
}
