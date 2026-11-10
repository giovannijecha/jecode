use super::*;
use crate::test_support::Directory;
use std::sync::{Arc, Barrier};

#[test]
fn concurrent_edits_preserve_both_independent_replacements() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("shared.txt");
    let content = format!("first=old\n{}\nsecond=old\n", "padding\n".repeat(65_536));
    fs::write(&path, content).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let handles: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|key| {
                let root = &root;
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    let arguments = Value::object([
                        ("path", Value::string("shared.txt")),
                        (
                            "edits",
                            Value::Array(vec![Value::object([
                                ("old_text", Value::string(format!("{key}=old"))),
                                ("new_text", Value::string(format!("{key}=new"))),
                            ])]),
                        ),
                    ]);
                    barrier.wait();
                    let result = edit(root, &arguments).unwrap();
                    assert!(result.get("error").is_none(), "{result:?}");
                })
            })
            .collect();
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let updated = fs::read_to_string(path).unwrap();
    assert!(updated.starts_with("first=new\n"));
    assert!(updated.ends_with("second=new\n"));
}

#[test]
fn a_stale_prepared_edit_preserves_an_external_writers_contents() {
    let directory = Directory::new();
    let path = directory.path().join("shared.txt");
    fs::write(&path, "external change").unwrap();
    let error = replace_file(
        &path,
        Some("old contents"),
        "stale replacement",
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(error.contains("file changed"), "{error}");
    assert_eq!(fs::read_to_string(path).unwrap(), "external change");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}
