use super::{TEXT_LIMIT, truncate_text};
use crate::browser::Registry;
use crate::cancel::Cancellation;
use crate::json::{self, Value};
use crate::scratch::Area;
use crate::test_support::Directory;

#[test]
fn snapshot_text_limit_counts_characters() {
    let mut text = "界".repeat(TEXT_LIMIT + 1);
    truncate_text(&mut text);
    assert_eq!(
        text,
        format!(
            "{}\n[truncated; full snapshot saved]",
            "界".repeat(TEXT_LIMIT)
        )
    );

    let mut exact = "界".repeat(TEXT_LIMIT);
    truncate_text(&mut exact);
    assert_eq!(exact, "界".repeat(TEXT_LIMIT));
}

#[test]
#[ignore = "requires an installed Chrome or Edge; isolated temporary profile"]
fn live_chromium_snapshot_values_text_and_refs() {
    let directory = Directory::new();
    let area = Area::new(
        directory.path().join("123456-789"),
        directory.path().into(),
        "123456-789",
    )
    .unwrap();
    let registry = Registry::default();
    let cancellation = Cancellation::default();
    let run = |action: &str, fields: Vec<(&str, Value)>| {
        let mut arguments = Value::object([("action", Value::string(action))]);
        if let Value::Object(values) = &mut arguments {
            for (key, value) in fields {
                values.insert(key.into(), value);
            }
        }
        registry.execute(&area, None, &arguments, &cancellation)
    };

    let opened = run("open", vec![]).unwrap();
    let tab = opened.get("tab_id").and_then(Value::as_str).unwrap();
    let script = r#"(()=>{
        document.body.innerHTML = '<input id="spaced" aria-label="Spaced" value="  Ada  "><input id="blank" aria-label="Blank" value="   "><input id="secret" type="password" aria-label="Secret" value="topsecret">';
        for (let i = 4; i <= 502; i++) {
            const input = document.createElement('input');
            input.id = 'item' + i;
            input.setAttribute('aria-label', 'Item ' + i);
            document.body.append(input);
        }
        const emoji = document.createElement('input');
        emoji.id = 'emoji';
        emoji.value = 'x'.repeat(199) + '😀';
        emoji.placeholder = 'x'.repeat(199) + '😀';
        document.body.append(emoji);
        document.body.append('界'.repeat(12001));
    })()"#;
    run(
        "evaluate",
        vec![
            ("tab_id", Value::string(tab)),
            ("expression", Value::string(script)),
        ],
    )
    .unwrap();

    let snapshot = run("snapshot", vec![("tab_id", Value::string(tab))]).unwrap();
    assert_eq!(snapshot.get("elements_total"), Some(&Value::number(503)));
    let text = snapshot.get("text").and_then(Value::as_str).unwrap();
    assert_eq!(
        text,
        format!(
            "{}\n[truncated; full snapshot saved]",
            "界".repeat(TEXT_LIMIT)
        )
    );
    let relative = snapshot
        .get("snapshot_path")
        .and_then(Value::as_str)
        .unwrap()
        .strip_prefix("tmp:")
        .unwrap();
    let full =
        json::parse(&std::fs::read_to_string(area.ensure().unwrap().join(relative)).unwrap())
            .unwrap();
    let elements = full.get("elements").and_then(Value::as_array).unwrap();
    assert_eq!(elements.len(), 503);
    assert_eq!(elements[0].get("value"), Some(&Value::string("  Ada  ")));
    assert_eq!(elements[1].get("value"), Some(&Value::string("   ")));
    assert!(elements[2].get("value").is_none());
    assert_eq!(elements[499].get("name"), Some(&Value::string("Item 500")));
    assert!(elements[500].get("name").is_none());
    assert_eq!(elements[500].get("ref"), Some(&Value::string("@501")));
    let emoji = format!("{}😀", "x".repeat(199));
    assert_eq!(elements[502].get("value"), Some(&Value::string(&emoji)));
    assert_eq!(
        elements[502].get("placeholder"),
        Some(&Value::string(&emoji))
    );
    assert_eq!(full.get("text"), Some(&Value::string("界".repeat(12001))));

    let check = run(
        "evaluate",
        vec![
            ("tab_id", Value::string(tab)),
            (
                "expression",
                Value::string("window[Symbol.for('jecode.browser.refs')].length"),
            ),
        ],
    )
    .unwrap();
    assert_eq!(check.get("value"), Some(&Value::number(503)));
    run(
        "fill",
        vec![
            ("tab_id", Value::string(tab)),
            ("selector", Value::string("@501")),
            ("text", Value::string("reference works")),
        ],
    )
    .unwrap();
    let filled = run(
        "evaluate",
        vec![
            ("tab_id", Value::string(tab)),
            (
                "expression",
                Value::string("document.getElementById('item501').value"),
            ),
        ],
    )
    .unwrap();
    assert_eq!(filled.get("value"), Some(&Value::string("reference works")));
}
