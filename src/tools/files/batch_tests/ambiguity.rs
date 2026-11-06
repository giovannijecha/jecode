use super::*;

#[test]
fn middle_ambiguity_returns_exact_windows_for_a_corrected_retry() {
    for padding in [5000, MAX_DIFF_BYTES / 2] {
        let directory = Directory::new();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("source.txt");
        let original = format!(
            "{}LEFT Ω MATCH end\n{}RIGHT 日本 MATCH end\n{}",
            "x".repeat(padding),
            "x".repeat(padding / 2),
            "x".repeat(padding)
        );
        fs::write(&path, &original).unwrap();
        let rejected = edit(&root, &batch(&[("MATCH", "changed")])).unwrap();
        let windows = rejected
            .get("match_context")
            .and_then(Value::as_array)
            .expect("two exact match windows");
        assert_eq!(windows.len(), 2);
        assert_eq!(
            rejected.get("match_context_source").and_then(Value::as_str),
            Some("original")
        );
        for (index, marker) in ["LEFT Ω MATCH end", "RIGHT 日本 MATCH end"]
            .iter()
            .enumerate()
        {
            let offset = windows[index]
                .get("offset")
                .and_then(Value::as_usize)
                .unwrap();
            let text = windows[index].get("text").and_then(Value::as_str).unwrap();
            assert_eq!(&original[offset..offset + "MATCH".len()], "MATCH");
            assert!(text.contains(marker), "{text:?}");
            assert!(text.len() <= 256);
        }
        let preview_bytes = rejected
            .get("current_content")
            .and_then(Value::as_str)
            .unwrap()
            .len()
            + rejected
                .get("current_tail")
                .and_then(Value::as_str)
                .map_or(0, str::len);
        let window_bytes: usize = windows
            .iter()
            .map(|window| window.get("text").and_then(Value::as_str).unwrap().len())
            .sum();
        assert!(preview_bytes + window_bytes <= 8192);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let corrected = edit(&root, &batch(&[("LEFT Ω MATCH end", "fixed")])).unwrap();
        assert!(corrected.get("error").is_none(), "{corrected:?}");
        assert!(fs::read_to_string(&path).unwrap().contains("fixed"));
    }
}

#[test]
fn later_ambiguity_windows_use_staged_text_and_include_overlaps() {
    let directory = Directory::new();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "anchor\nother MATCH\n").unwrap();
    let rejected = edit(&root, &batch(&[("anchor", "anchor MATCH"), ("MATCH", "x")])).unwrap();
    assert_eq!(
        rejected.get("match_context_source").and_then(Value::as_str),
        Some("staged")
    );
    assert_eq!(
        rejected.get("current_content").and_then(Value::as_str),
        Some("anchor\nother MATCH\n")
    );
    let windows = rejected
        .get("match_context")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(windows.len(), 2);
    assert!(
        windows[0]
            .get("text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("anchor MATCH")
    );
    assert!(
        windows[1]
            .get("text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("other MATCH")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "anchor\nother MATCH\n");
    let corrected = edit(
        &root,
        &batch(&[("anchor", "anchor MATCH"), ("anchor MATCH", "fixed")]),
    )
    .unwrap();
    assert!(corrected.get("error").is_none(), "{corrected:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "fixed\nother MATCH\n");

    fs::write(&path, "aaaa").unwrap();
    let overlap = edit(&root, &batch(&[("aa", "x")])).unwrap();
    let windows = overlap
        .get("match_context")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(windows[0].get("offset").and_then(Value::as_usize), Some(0));
    assert_eq!(windows[1].get("offset").and_then(Value::as_usize), Some(1));
    assert_eq!(fs::read_to_string(&path).unwrap(), "aaaa");

    let utf8 = format!("{}AB MATCH|{}CD MATCH", "Ω".repeat(300), "日本".repeat(200));
    fs::write(&path, &utf8).unwrap();
    let rejected = edit(&root, &batch(&[("MATCH", "x")])).unwrap();
    let windows = rejected
        .get("match_context")
        .and_then(Value::as_array)
        .unwrap();
    for window in windows {
        let start = window
            .get("excerpt_byte_offset")
            .and_then(Value::as_usize)
            .unwrap();
        let text = window.get("text").and_then(Value::as_str).unwrap();
        assert!(utf8.is_char_boundary(start));
        assert_eq!(&utf8[start..start + text.len()], text);
        assert!(text.contains("MATCH"));
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), utf8);
}
