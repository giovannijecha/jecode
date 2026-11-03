//! Opt-in interoperability with explicitly selected, externally installed servers.
use super::*;
use crate::test_support::Directory;
use std::time::{Duration, Instant};

fn call(
    tools: &Tools,
    profile: &Profile,
    action: &str,
    path: &str,
    position: Option<(usize, usize)>,
) -> Value {
    let mut arguments = Value::object([
        ("action", Value::string(action)),
        ("server", Value::string(&profile.id)),
        ("path", Value::string(path)),
    ]);
    if let (Some((line, column)), Value::Object(fields)) = (position, &mut arguments) {
        fields.insert("line".into(), Value::number(line));
        fields.insert("column".into(), Value::number(column));
        if action == "references" {
            fields.insert("include_declaration".into(), Value::Bool(true));
        }
    }
    tools.execute("lsp", &arguments.encode())
}

fn diagnostics_until(
    tools: &Tools,
    profile: &Profile,
    path: &str,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let started = Instant::now();
    loop {
        let result = call(tools, profile, "diagnostics", path, None);
        if predicate(&result) {
            return result;
        }
        assert!(
            started.elapsed() < Duration::from_secs(45),
            "Diagnostic state did not converge: {}",
            result.pretty()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn semantic_case(
    mut profile: Profile,
    main_path: &str,
    clean: &str,
    broken: &str,
    prepare: impl Fn(&Path),
    dependency: (&str, &str, &str),
) {
    let directory = Directory::new();
    let root = directory.path().join("LSP spaces 🦀");
    std::fs::create_dir(&root).unwrap();
    prepare(&root);
    std::fs::write(root.join(main_path), clean).unwrap();
    let home = Directory::new();
    profile.request_timeout_ms = 30_000;
    Store::new(home.path().to_path_buf())
        .save(profile.clone())
        .unwrap();
    let mut tools = Tools::new(&root).unwrap();
    tools.configure_lsp(home.path(), Redactor::empty()).unwrap();
    let usage_line = clean
        .lines()
        .position(|line| line.contains("return answer()") || line.contains("{ answer() }"))
        .unwrap();
    let usage = clean.lines().nth(usage_line).unwrap();
    let byte = usage.find("answer()").unwrap();
    let position = (usage_line + 1, usage[..byte].chars().count() + 1);
    let cold = Instant::now();
    let definition = loop {
        let result = call(&tools, &profile, "definition", main_path, Some(position));
        if result
            .get("locations")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
        {
            break result;
        }
        assert!(
            cold.elapsed() < Duration::from_secs(45),
            "No indexed definition: {}",
            result.pretty()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        !definition
            .get("locations")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .is_empty(),
        "{}",
        definition.pretty()
    );
    eprintln!("{} cold definition: {:?}", profile.name, cold.elapsed());
    let generation = definition.get("instance_generation").cloned();
    let warm = Instant::now();
    let references = call(&tools, &profile, "references", main_path, Some(position));
    assert!(
        !references
            .get("locations")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .is_empty(),
        "{}",
        references.pretty()
    );
    assert_eq!(generation, references.get("instance_generation").cloned());
    eprintln!("{} warm references: {:?}", profile.name, warm.elapsed());
    let clean_report = diagnostics_until(&tools, &profile, main_path, |value| {
        value.get("freshness").and_then(Value::as_str) == Some("current")
            && value
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|items| items.is_empty())
    });
    assert_eq!(clean_report.get("complete"), Some(&Value::Bool(true)));
    // External on-disk changes must be observed without a ToolChanged event.
    std::fs::write(root.join(main_path), broken).unwrap();
    let errors = diagnostics_until(&tools, &profile, main_path, |value| {
        value.get("freshness").and_then(Value::as_str) == Some("current")
            && value
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item.get("severity").and_then(Value::as_usize) == Some(1))
                })
    });
    eprintln!(
        "{} introduced diagnostics: {}",
        profile.name,
        errors.pretty()
    );
    let updated = broken.replace("missing()", "another_missing()");
    let written = tools.execute(
        "write",
        &Value::object([
            ("path", Value::string(main_path)),
            ("content", Value::string(updated)),
        ])
        .encode(),
    );
    assert!(
        written.get("bytes_written").is_some(),
        "{}",
        written.pretty()
    );
    diagnostics_until(&tools, &profile, main_path, |value| {
        value.get("freshness").and_then(Value::as_str) == Some("current")
            && value
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item.get("message")
                            .and_then(Value::as_str)
                            .is_some_and(|text| text.contains("another_missing"))
                    })
                })
    });
    std::fs::write(
        root.join(main_path),
        broken.replace("missing()", "answer()"),
    )
    .unwrap();
    let warning = diagnostics_until(&tools, &profile, main_path, |value| {
        value.get("freshness").and_then(Value::as_str) == Some("current")
            && value
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item.get("severity").and_then(Value::as_usize) == Some(2))
                        && !items
                            .iter()
                            .any(|item| item.get("severity").and_then(Value::as_usize) == Some(1))
                })
    });
    eprintln!("{} updated to warning: {}", profile.name, warning.pretty());
    std::fs::write(root.join(main_path), clean).unwrap();
    let cleared = diagnostics_until(&tools, &profile, main_path, |value| {
        value.get("freshness").and_then(Value::as_str) == Some("current")
            && value
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|items| items.is_empty())
    });
    assert_eq!(generation, cleared.get("instance_generation").cloned());
    std::fs::write(root.join(dependency.0), dependency.1).unwrap();
    let cross_file = diagnostics_until(&tools, &profile, main_path, |value| {
        // rust-analyzer can publish dependency diagnostics without a version.
        // Keep that uncertainty visible while verifying the actual transition.
        matches!(
            value.get("freshness").and_then(Value::as_str),
            Some("current" | "unverified")
        ) && value
            .get("diagnostics")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.get("severity").and_then(Value::as_usize) == Some(1))
            })
    });
    assert_eq!(
        std::fs::read_to_string(root.join(main_path)).unwrap(),
        clean,
        "Reanalysis must not modify the unchanged source file"
    );
    assert_eq!(generation, cross_file.get("instance_generation").cloned());
    std::fs::write(root.join(dependency.0), dependency.2).unwrap();
    diagnostics_until(&tools, &profile, main_path, |value| {
        matches!(
            value.get("freshness").and_then(Value::as_str),
            Some("current" | "unverified")
        ) && value
            .get("diagnostics")
            .and_then(Value::as_array)
            .is_some_and(|items| items.is_empty())
    });
    tools.stop_processes();
}

#[test]
#[ignore = "requires JECODE_LSP_CLANGD with an explicitly selected clangd executable"]
fn real_clangd_navigation_and_diagnostics() {
    let mut profile = Profile::draft_guided_cpp();
    profile.executable = std::env::var("JECODE_LSP_CLANGD").expect("select clangd explicitly");
    profile.arguments = vec![
        "--background-index=false".into(),
        "--pch-storage=memory".into(),
    ];
    semantic_case(
        profile,
        "main.cpp",
        "#include \"answer.hpp\"\r\n/* 🦀 */ int main() { return answer(); }\r\n",
        "#include \"answer.hpp\"\r\n/* 🦀 */ int main() { int unused = 1; return missing(); }\r\n",
        |root| {
            std::fs::write(
                root.join("answer.hpp"),
                "inline int answer() { return 42; }\r\n",
            )
            .unwrap();
            let compile = Value::Array(vec![Value::object([
                ("directory", Value::string(root.to_string_lossy())),
                ("file", Value::string("main.cpp")),
                (
                    "arguments",
                    Value::Array(
                        [
                            "clang++",
                            "-std=c++17",
                            "-Wall",
                            "-Wextra",
                            "-c",
                            "main.cpp",
                        ]
                        .map(Value::string)
                        .to_vec(),
                    ),
                ),
            ])]);
            std::fs::write(root.join("compile_commands.json"), compile.encode()).unwrap();
        },
        (
            "answer.hpp",
            "inline const char* answer() { return \"bad\"; }\r\n",
            "inline int answer() { return 42; }\r\n",
        ),
    );
}

#[test]
#[ignore = "requires JECODE_LSP_RUST with an explicitly selected rust-analyzer executable"]
fn real_rust_navigation_and_diagnostics() {
    let mut profile = Profile::draft_guided_rust();
    profile.executable = std::env::var("JECODE_LSP_RUST").expect("select rust-analyzer explicitly");
    if let Ok(path) = std::env::var("JECODE_LSP_RUST_PATH") {
        profile.environment.insert("PATH".into(), path);
    }
    profile
        .environment
        .insert("CARGO_NET_OFFLINE".into(), "true".into());
    profile.environment.insert(
        "RUSTUP_TOOLCHAIN".into(),
        std::env::var("JECODE_LSP_RUST_TOOLCHAIN").unwrap_or_else(|_| "1.95.0".into()),
    );
    profile.initialization_options = crate::json::parse(r#"{"cargo":{"sysroot":null,"buildScripts":{"enable":false}},"procMacro":{"enable":false},"check":{"extraArgs":["--offline"]}}"#).unwrap();
    semantic_case(
        profile,
        "lib.rs",
        "mod helper; pub fn answer() -> i32 { helper::answer() }\r\n/* 🦀 */ pub fn consumer() -> i32 { answer() }\r\n",
        "mod helper; pub fn answer() -> i32 { helper::answer() }\r\n/* 🦀 */ pub fn consumer() -> i32 { let unused = 1; missing() }\r\n",
        |root| {
            std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"jecode_lsp_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n").unwrap();
            std::fs::write(root.join("helper.rs"), "pub fn answer() -> i32 { 42 }\r\n").unwrap();
        },
        (
            "helper.rs",
            "pub fn answer() -> &'static str { \"bad\" }\r\n",
            "pub fn answer() -> i32 { 42 }\r\n",
        ),
    );
}
