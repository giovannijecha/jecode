use super::*;
use std::io::{Read, Write};

const PREFIX: u64 = (12 << 20) - 2;
const ORIGINAL: &str = "é日🦀-unique-marker";
const UPDATED: &str = "replacement-with-a-different-length";
const SUFFIX: &[u8] = b"\nthe exact trailing bytes\r\n";

fn verify(path: &Path, marker: &str) {
    let mut file = fs::File::open(path).unwrap();
    let mut remaining = PREFIX;
    let mut block = [0; 64 * 1024];
    while remaining > 0 {
        let count = remaining.min(block.len() as u64) as usize;
        file.read_exact(&mut block[..count]).unwrap();
        assert!(block[..count].iter().all(|byte| *byte == b'x'));
        remaining -= count as u64;
    }
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, [marker.as_bytes(), SUFFIX].concat());
}

#[test]
fn large_text_edit_history_paging_restore_and_undo_are_exact() {
    let fixture = fixture();
    let path = fixture.root.path().join("large.txt");
    let mut file = fs::File::create(&path).unwrap();
    let block = [b'x'; 64 * 1024];
    let mut remaining = PREFIX;
    while remaining > 0 {
        let count = remaining.min(block.len() as u64) as usize;
        file.write_all(&block[..count]).unwrap();
        remaining -= count as u64;
    }
    file.write_all(ORIGINAL.as_bytes()).unwrap();
    file.write_all(SUFFIX).unwrap();
    drop(file);
    let result = call(
        &fixture.tools,
        "edit",
        &[
            ("path", Value::string("large.txt")),
            ("old_text", Value::string(ORIGINAL)),
            ("new_text", Value::string(UPDATED)),
        ],
    );
    assert_eq!(
        text(&result, &["checkpoint", "status"]),
        "saved",
        "{result:?}"
    );
    verify(&path, UPDATED);
    let before = text(&result, &["checkpoint", "before"]);
    let reference = format!("checkpoint:{before}:large.txt");
    let page = call(
        &fixture.tools,
        "read",
        &[
            ("path", Value::string(&reference)),
            ("byte_offset", Value::number(PREFIX)),
        ],
    );
    assert_eq!(
        text(&page, &["content"]),
        format!("{ORIGINAL}{}", std::str::from_utf8(SUFFIX).unwrap())
    );
    assert_eq!(page.get("truncated"), Some(&Value::Bool(false)));
    let restored = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(before)),
            ("paths", Value::Array(vec![Value::string("large.txt")])),
        ],
    );
    verify(&path, ORIGINAL);
    let undo = text(&restored, &["undo", "ref"]);
    let undone = call(
        &fixture.tools,
        "checkpoint",
        &[
            ("action", Value::string("restore")),
            ("ref", Value::string(undo)),
            ("paths", Value::Array(vec![Value::string("large.txt")])),
        ],
    );
    assert!(undone.get("error").is_none(), "{undone:?}");
    verify(&path, UPDATED);
    assert_eq!(
        fs::read(fixture.root.path().join("notes.txt")).unwrap(),
        b"first\n"
    );

    struct ExactText {
        position: u64,
    }
    impl Write for ExactText {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            for byte in bytes {
                let expected = if self.position < PREFIX {
                    b'x'
                } else {
                    let at = (self.position - PREFIX) as usize;
                    [ORIGINAL.as_bytes(), SUFFIX].concat()[at]
                };
                assert_eq!(*byte, expected);
                self.position += 1;
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = ExactText { position: 0 };
    assert!(
        crate::checkpoints::command::stream_show(
            fixture._home.path(),
            fixture.root.path(),
            &["show".into(), before.into(), "large.txt".into(),],
            &mut output
        )
        .unwrap()
    );
    assert_eq!(
        output.position,
        PREFIX + ORIGINAL.len() as u64 + SUFFIX.len() as u64
    );
}
