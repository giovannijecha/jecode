use super::*;
use crate::test_support::Directory;
use crate::tools::Begun;

fn arguments(fields: &[(&str, &str)]) -> String {
    Value::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).into(), Value::string(*value)))
            .collect(),
    )
    .encode()
}

#[test]
fn file_tools_are_inside_unless_the_resolved_path_leaves_the_folder() {
    let project = Directory::new();
    let other = Directory::new();
    fs::write(project.path().join("a.rs"), "inside").unwrap();
    fs::write(other.path().join("b.rs"), "outside").unwrap();
    let tools = Tools::new(project.path()).unwrap();

    let inside = tools
        .access("read", &arguments(&[("path", "a.rs")]))
        .unwrap();
    assert_eq!(inside.kind, Kind::Reads);
    assert!(!inside.outside && inside.base.is_none());

    let outside_path = other.path().join("b.rs");
    let outside = tools
        .access(
            "read",
            &arguments(&[("path", &outside_path.to_string_lossy())]),
        )
        .unwrap();
    assert!(outside.outside);
    let canonical = fs::canonicalize(other.path()).unwrap();
    assert_eq!(outside.base.as_deref(), Some(canonical.as_path()));
    assert_eq!(outside.target, plain(&canonical.join("b.rs")));

    // A new file resolves through its parent; `..` cannot hide the exit.
    let escape = format!(
        "../{}/new.rs",
        other.path().file_name().unwrap().to_string_lossy()
    );
    let created = tools
        .access("write", &arguments(&[("path", &escape), ("content", "x")]))
        .unwrap();
    assert_eq!(created.kind, Kind::Edits);
    assert!(created.outside);

    // A folder listed outside becomes the root itself.
    let listed = tools
        .access(
            "list",
            &arguments(&[("path", &other.path().to_string_lossy())]),
        )
        .unwrap();
    assert_eq!(listed.base.as_deref(), Some(canonical.as_path()));
    assert!(!tools.access("list", "{}").unwrap().outside);
}

#[test]
fn commands_are_outside_only_through_an_exec_cwd() {
    let project = Directory::new();
    let other = Directory::new();
    let tools = Tools::new(project.path()).unwrap();

    let bash = tools
        .access("bash", &arguments(&[("command", "cd / && ls")]))
        .unwrap();
    assert_eq!((bash.kind, bash.outside), (Kind::Runs, false));

    let exec = Value::object([
        ("executable", Value::string("cargo")),
        ("args", Value::Array(vec![Value::string("test")])),
        ("cwd", Value::string(other.path().to_string_lossy())),
    ])
    .encode();
    let access = tools.access("exec", &exec).unwrap();
    assert!(access.outside);
    assert_eq!(access.target, "cargo test");
    assert_eq!(
        access.place,
        Some(plain(&fs::canonicalize(other.path()).unwrap()))
    );
}

#[test]
fn outside_exec_grants_use_exact_invocation_identity_without_exposing_inputs() {
    let project = Directory::new();
    let outside = Directory::new();
    let tools = Tools::new(project.path()).unwrap();
    let make = |args: Vec<&str>, env: &str, stdin: &str| {
        Value::object([
            ("executable", Value::string("fixture")),
            (
                "args",
                Value::Array(args.into_iter().map(Value::string).collect()),
            ),
            ("env", Value::object([("MODE", Value::string(env))])),
            ("stdin", Value::string(stdin)),
            ("cwd", Value::string(outside.path().to_string_lossy())),
        ])
        .encode()
    };
    let original = tools
        .access("exec", &make(vec!["a b", "c"], "one", "private input"))
        .unwrap();
    assert!(original.outside);
    let identity = original.identity.as_deref().unwrap();
    assert_eq!(identity.len(), 64);
    assert!(!identity.contains("private input"));
    let same = tools
        .access("exec", &make(vec!["a b", "c"], "one", "private input"))
        .unwrap();
    assert_eq!(original.grant(), same.grant());
    for variant in [
        make(vec!["a", "b c"], "one", "private input"),
        make(vec!["a b", "c"], "two", "private input"),
        make(vec!["a b", "c"], "one", "different input"),
    ] {
        let changed = tools.access("exec", &variant).unwrap();
        assert_ne!(original.grant(), changed.grant());
    }
}

#[test]
fn the_workspace_tree_is_recursive_but_a_sibling_is_outside() {
    let project = Directory::new();
    let tools = Tools::new(project.path()).unwrap();
    let nested = project.path().join("one/two");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("file.rs"), "inside").unwrap();
    for (name, path) in [("read", "one/two/file.rs"), ("write", "one/two/new.rs")] {
        let access = tools.access(name, &arguments(&[("path", path)])).unwrap();
        assert!(!access.outside, "{name} {path}");
    }
    let exec = Value::object([
        ("executable", Value::string("cargo")),
        ("cwd", Value::string("one/two")),
    ]);
    assert!(!tools.access("exec", &exec.encode()).unwrap().outside);

    let sibling = Directory::new();
    let path = sibling.path().join("file.rs");
    fs::write(&path, "outside").unwrap();
    let outside = tools
        .access("read", &arguments(&[("path", &path.to_string_lossy())]))
        .unwrap();
    assert!(outside.outside);
}

#[test]
fn read_blocks_direct_project_mutations_outside_file_tools() {
    let project = Directory::new();
    let tools = Tools::new(project.path()).unwrap();
    let permissions = crate::permissions::Permissions::default();
    permissions
        .set_level(crate::permissions::Level::Read)
        .unwrap();
    let isolated = Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([(
                "workspace",
                Value::string("isolated"),
            )])]),
        ),
    ]);
    for (name, args) in [
        (
            "agent",
            Value::object([("action", Value::string("integrate"))]),
        ),
        ("agent", Value::object([("action", Value::string("close"))])),
        ("agent", isolated),
        (
            "process",
            Value::object([("action", Value::string("write"))]),
        ),
    ] {
        let access = tools.access(name, &args.encode()).expect(name);
        assert!(
            matches!(
                permissions.check(&access),
                crate::permissions::Verdict::Unavailable(_)
            ),
            "{name}"
        );
    }
    let shared = Value::object([
        ("action", Value::string("create")),
        (
            "tasks",
            Value::Array(vec![Value::object([(
                "workspace",
                Value::string("shared"),
            )])]),
        ),
    ]);
    assert!(tools.access("agent", &shared.encode()).is_none());
}

#[test]
fn read_blocks_explicit_mutating_http_methods() {
    let project = Directory::new();
    let tools = Tools::new(project.path()).unwrap();
    let permissions = crate::permissions::Permissions::default();
    permissions
        .set_level(crate::permissions::Level::Read)
        .unwrap();

    for (method, body, blocked) in [
        (None, None, false),
        (Some("GET"), None, false),
        (Some("HEAD"), None, false),
        (Some("head"), None, true),
        (Some("POST"), None, true),
        (Some("PATCH"), None, true),
        (Some("DELETE"), None, true),
        (Some("GET"), Some("payload"), true),
        (None, Some("payload"), true),
    ] {
        let mut fields = vec![("action".into(), Value::string("fetch"))];
        if let Some(method) = method {
            fields.push(("method".into(), Value::string(method)));
        }
        if let Some(body) = body {
            fields.push(("body".into(), Value::string(body)));
        }
        let access = tools.access("web", &Value::Object(fields.into_iter().collect()).encode());
        assert_eq!(access.is_some(), blocked, "{method:?} {body:?}");
        if let Some(access) = access {
            assert!(matches!(
                permissions.check(&access),
                crate::permissions::Verdict::Unavailable(_)
            ));
        }
    }
    assert!(
        tools
            .access("web", &arguments(&[("action", "crawl")]))
            .is_none()
    );
}

#[test]
fn jecode_areas_and_ungoverned_tools_need_no_level() {
    let project = Directory::new();
    let tools = Tools::new(project.path()).unwrap();
    assert!(
        tools
            .access("write", &arguments(&[("path", "tmp:notes.md")]))
            .is_none()
    );
    let reference = tools
        .access("read", &arguments(&[("path", "output:abc")]))
        .unwrap();
    assert!(!reference.outside);
    for name in [
        "web",
        "browser",
        "process",
        "agent",
        "todo_list",
        "ask_user",
    ] {
        assert!(tools.access(name, "{}").is_none(), "{name}");
    }
    assert!(
        tools
            .access("checkpoint", &arguments(&[("action", "list")]))
            .is_none()
    );
    assert_eq!(
        tools
            .access("checkpoint", &arguments(&[("action", "restore")]))
            .unwrap()
            .kind,
        Kind::Edits
    );
}

#[test]
fn an_approved_outside_base_lets_file_tools_work_there() {
    let project = Directory::new();
    let other = Directory::new();
    fs::write(other.path().join("b.rs"), "outside\n").unwrap();
    let tools = Tools::new(project.path()).unwrap();
    let path = other.path().join("b.rs").to_string_lossy().into_owned();
    let read = arguments(&[("path", &path)]);

    let refused = tools.execute("read", &read);
    assert!(refused.get("error").is_some(), "{refused:?}");

    let access = tools.access("read", &read).unwrap();
    let begun = Begun::default().with_base(access.base);
    let allowed = tools
        .execute_begun(
            begun,
            "read",
            &read,
            &crate::cancel::Cancellation::default(),
        )
        .result;
    assert!(allowed.encode().contains("outside"), "{allowed:?}");
}

#[test]
fn an_approved_relative_outside_path_reaches_the_approved_file() {
    let parent = Directory::new();
    let project = parent.path().join("project");
    let deep = parent.path().join("other").join("deep");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("f.txt"), "relative outside\n").unwrap();
    let tools = Tools::new(&project).unwrap();
    let cancellation = crate::cancel::Cancellation::default();
    let run = |name: &str, arguments: &str| {
        let access = tools.access(name, arguments).unwrap();
        assert!(access.outside);
        let begun = Begun::default().with_base(access.base);
        tools
            .execute_begun(begun, name, arguments, &cancellation)
            .result
    };

    let read = run("read", &arguments(&[("path", "../other/deep/f.txt")]));
    assert!(read.encode().contains("relative outside"), "{read:?}");
    let written = run(
        "write",
        &arguments(&[("path", "../other/deep/new.txt"), ("content", "made\n")]),
    );
    assert!(written.get("error").is_none(), "{written:?}");
    assert_eq!(fs::read_to_string(deep.join("new.txt")).unwrap(), "made\n");
    let listed = run("list", &arguments(&[("path", "../other/deep")]));
    assert!(listed.encode().contains("f.txt"), "{listed:?}");
}
