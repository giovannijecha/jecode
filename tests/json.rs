use jecode::json::{self, Error, Limits, Value};

fn parse(text: &str) -> Result<Value, Error> {
    json::parse(text, Limits::default())
}

#[test]
fn unicode_controls_numbers_and_nested_values_round_trip() {
    let value = parse(r#"{"text":"héllo\n\uD83D\uDE80\u0000\b\f\r\t\\\/\"","array":[null,true,false,-0,1.25e+10,18446744073709551615]}"#).unwrap();
    assert_eq!(
        value.get("text").unwrap().text(),
        Some("héllo\n🚀\0\x08\x0c\r\t\\/\"")
    );
    assert_eq!(
        value.get("array").unwrap().array().unwrap()[5].unsigned(),
        Some(u64::MAX)
    );
    assert_eq!(parse(&json::encode(&value, 4096).unwrap()), Ok(value));
}

#[test]
fn rejects_ambiguous_malformed_and_incomplete_input() {
    for text in [
        "",
        "[1,]",
        "{\"a\":1,}",
        "00",
        "01",
        "-",
        "1.",
        "1e",
        "1e+",
        "+1",
        "NaN",
        "true false",
        "[",
        "{",
        "\"",
        "\"\n\"",
        r#""\x00""#,
        r#""\uD800""#,
        r#""\uDC00""#,
        r#""\uD800\u0041""#,
        r#""\uQQQQ""#,
    ] {
        assert!(parse(text).is_err(), "accepted {text:?}");
    }
    assert_eq!(parse(r#"{"a":1,"\u0061":2}"#), Err(Error::DuplicateKey));
    assert_eq!(parse("18446744073709551616").unwrap().unsigned(), None);
    assert_eq!(parse("1.0").unwrap().unsigned(), None);
}

#[test]
fn parser_and_writer_enforce_budgets() {
    for limits in [
        Limits {
            bytes: 2,
            ..Default::default()
        },
        Limits {
            nodes: 1,
            ..Default::default()
        },
        Limits {
            depth: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(json::parse("[1]", limits), Err(Error::Limit));
    }
    assert_eq!(
        json::encode(&Value::String("\n".into()), 3),
        Err(Error::Limit)
    );
    assert_eq!(
        json::encode(&Value::Number("1,null".into()), 100),
        Err(Error::Syntax)
    );
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert_eq!(parse(&deep), Err(Error::Limit));
    assert!(parse(&format!("{}0{}", "[".repeat(64), "]".repeat(64))).is_ok());
}

#[test]
fn arbitrary_utf8_prefixes_do_not_panic_or_silently_repair() {
    let source = r#"{"𐐀":"\uD83D\uDE80é","x":[-12.5e-3,true]}"#;
    for end in (0..source.len()).filter(|n| source.is_char_boundary(*n)) {
        assert!(parse(&source[..end]).is_err());
    }
    for code in 0..128 {
        let character = char::from_u32(code).unwrap();
        let value = Value::String(character.to_string());
        assert_eq!(parse(&json::encode(&value, 32).unwrap()), Ok(value));
    }
}
