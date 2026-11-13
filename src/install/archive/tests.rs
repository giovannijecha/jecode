use super::*;

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
        .collect()
}

/// SHA-256 and length of the text the fixture generator compressed with level 9.
const DYNAMIC_TEXT: (&str, usize) = (
    "891e979b6c219c9f1589a0008bd04dd93935e483dff7f6804555d0698397161a",
    3000,
);

const DYNAMIC: &str = include_str!("tests/dynamic.hex");

#[test]
fn gzip_members_with_stored_fixed_and_dynamic_blocks_inflate_exactly() {
    let short = b"hello hello hello jecode\n";
    let stored = "1f8b080000000000040a011900e6ff68656c6c6f2068656c6c6f2068656c6c6f206a65636f64650a84ee96db19000000";
    let fixed = "1f8b080000000000020acb48cdc9c957c84022b35293f35352b90084ee96db19000000";
    assert_eq!(gunzip(&bytes(stored), 1024).unwrap(), short);
    assert_eq!(gunzip(&bytes(fixed), 1024).unwrap(), short);
    let dynamic = bytes(DYNAMIC.trim());
    assert_eq!(
        (dynamic[10] >> 1) & 3,
        2,
        "the fixture uses a dynamic block"
    );
    let text = gunzip(&dynamic, 1 << 20).unwrap();
    assert_eq!(text.len(), DYNAMIC_TEXT.1);
    assert_eq!(
        crate::permissions::identity::sha256_hex(&text).unwrap(),
        DYNAMIC_TEXT.0
    );
}

#[test]
fn corrupt_truncated_and_oversized_streams_are_rejected() {
    let fixed = bytes("1f8b080000000000020acb48cdc9c957c84022b35293f35352b90084ee96db19000000");
    let mut corrupt = fixed.clone();
    let last = corrupt.len() - 5;
    corrupt[last] ^= 0xff;
    assert!(gunzip(&corrupt, 1024).unwrap_err().contains("checksum"));
    assert!(gunzip(&fixed[..fixed.len() - 9], 1024).is_err());
    assert!(gunzip(&fixed, 8).unwrap_err().contains("limit"));
    assert!(gunzip(b"not gzip at all, really", 1024).is_err());
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

fn header(name: &str, kind: u8, size: usize) -> Vec<u8> {
    let mut header = vec![0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..107].copy_from_slice(b"0000644");
    header[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
    header[156] = kind;
    header[257..263].copy_from_slice(b"ustar\0");
    header[148..156].fill(b' ');
    let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    header
}

pub fn tar(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    for (name, kind, content) in entries {
        archive.extend(header(name, *kind, content.len()));
        archive.extend_from_slice(content);
        archive.resize(archive.len().div_ceil(512) * 512, 0);
    }
    archive.extend([0u8; 1024]);
    archive
}

#[test]
fn ustar_returns_regular_files_and_refuses_links_and_traversal() {
    let archive = tar(&[
        ("package/", b'5', b""),
        ("package/package.json", b'0', b"{}"),
        ("package/native/linux-x64/jecode", b'0', &[7u8; 700]),
    ]);
    let files = untar(&archive).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files["package/native/linux-x64/jecode"], vec![7u8; 700]);
    assert!(untar(&tar(&[("package/link", b'2', b"")])).is_err());
    assert!(untar(&tar(&[("package/../evil", b'0', b"x")])).is_err());
    assert!(
        untar(&tar(&[
            ("package/a", b'0', b"x"),
            ("package/a", b'0', b"y")
        ]))
        .is_err()
    );
    let mut damaged = tar(&[("package/a", b'0', b"x")]);
    damaged[0] = b'q';
    assert!(untar(&damaged).unwrap_err().contains("checksum"));
}
