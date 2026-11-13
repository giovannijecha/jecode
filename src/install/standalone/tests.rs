use super::*;
use crate::test_support::Directory;
use std::fs;

/// A gzip member of stored DEFLATE blocks, enough to build fixture tarballs.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut output = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(65_535).collect()
    };
    for (index, chunk) in chunks.iter().enumerate() {
        output.push(u8::from(index + 1 == chunks.len()));
        let length = chunk.len() as u16;
        output.extend(length.to_le_bytes());
        output.extend((!length).to_le_bytes());
        output.extend_from_slice(chunk);
    }
    output.extend(crate::install::archive::crc32(data).to_le_bytes());
    output.extend((data.len() as u32).to_le_bytes());
    output
}

pub fn tarball(version: &str, binary: &[u8]) -> Vec<u8> {
    let (key, name) = provenance::target();
    let package = format!(r#"{{"name":"@giovannijecha/jecode","version":"{version}"}}"#);
    let manifest = format!(
        r#"{{"format":1,"version":"{version}","artifacts":[{{"target":"{key}","version":"{version}","sha256":"{}"}}]}}"#,
        crate::permissions::identity::sha256_hex(binary).unwrap()
    );
    let path = format!("package/native/{key}/{name}");
    gzip(&crate::install::archive::tests::tar(&[
        ("package/package.json", b'0', package.as_bytes()),
        ("package/native/manifest.json", b'0', manifest.as_bytes()),
        (&path, b'0', binary),
    ]))
}

fn release(version: &str, tarball: &[u8]) -> Release {
    Release {
        version: version.into(),
        tarball: "https://registry.example/jecode.tgz".into(),
        sha512: crate::install::sha512::digest(tarball).to_vec(),
    }
}

#[test]
fn release_executable_needs_matching_version_and_manifest_digest() {
    let archive = tarball("9.9.9", b"new executable");
    let found = executable(&archive, &release("9.9.9", &archive)).unwrap();
    assert_eq!(found, b"new executable");
    assert!(executable(&archive, &release("9.9.8", &archive)).is_err());
    let (key, name) = provenance::target();
    let tampered = gzip(&crate::install::archive::tests::tar(&[
        (
            "package/package.json",
            b'0',
            br#"{"name":"@giovannijecha/jecode","version":"9.9.9"}"#,
        ),
        (
            "package/native/manifest.json",
            b'0',
            format!(r#"{{"artifacts":[{{"target":"{key}","version":"9.9.9","sha256":"00"}}]}}"#)
                .as_bytes(),
        ),
        (&format!("package/native/{key}/{name}"), b'0', b"other"),
    ]));
    assert!(
        executable(&tampered, &release("9.9.9", &tampered))
            .unwrap_err()
            .contains("manifest")
    );
}

fn installation(directory: &Directory) -> (PathBuf, PathBuf) {
    let root = directory.path().join("install root");
    let binary = root.join("bin").join(provenance::target().1);
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, b"old executable").unwrap();
    fs::write(root.join(MARKER), "Jecode local Rust installation\n").unwrap();
    (root, binary)
}

#[test]
fn failed_validation_keeps_the_installed_executable_and_removes_the_stage() {
    let directory = Directory::new();
    let (_, binary) = installation(&directory);
    let error = replace(&binary, b"broken", |_| Err("refused".into())).unwrap_err();
    assert_eq!(error, "refused");
    assert_eq!(fs::read(&binary).unwrap(), b"old executable");
    assert!(leftovers(binary.parent().unwrap()).is_empty());
}

#[test]
fn successful_replacement_activates_the_validated_stage() {
    let directory = Directory::new();
    let (_, binary) = installation(&directory);
    let bin = binary.parent().unwrap().to_path_buf();
    let stale = bin.join(format!(
        ".jecode-previous-1-2{}",
        if cfg!(windows) { ".exe" } else { "" }
    ));
    fs::write(&stale, b"stale").unwrap();
    let previous = replace(&binary, b"new executable", |staged| {
        assert_eq!(fs::read(staged).unwrap(), b"new executable");
        Ok(())
    })
    .unwrap();
    assert_eq!(fs::read(&binary).unwrap(), b"new executable");
    assert!(!stale.exists());
    if cfg!(windows) {
        assert_eq!(fs::read(&previous[0]).unwrap(), b"old executable");
    } else {
        assert!(previous.is_empty());
    }
    assert!(system::remove_later(&previous, &[]).is_empty());
    assert_eq!(fs::read_dir(&bin).unwrap().count(), 1);
}

#[test]
fn uninstall_removes_owned_files_and_keeps_everything_else() {
    let directory = Directory::new();
    let (root, binary) = installation(&directory);
    fs::write(root.join("notes.txt"), b"unrelated").unwrap();
    let bin = binary.parent().unwrap();
    fs::write(
        bin.join(format!(
            ".jecode-update-3-4{}",
            if cfg!(windows) { ".exe" } else { "" }
        )),
        b"stage",
    )
    .unwrap();
    fs::write(bin.join(".jecode-update-not-ours"), b"unrelated").unwrap();
    let other = directory.path().join("other/bin");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join(provenance::target().1), b"another installation").unwrap();
    let outcome = uninstall(&root, &binary, None).unwrap();
    assert!(!binary.exists());
    assert!(!root.join(MARKER).exists());
    assert_eq!(
        fs::read(root.join("bin/.jecode-update-not-ours")).unwrap(),
        b"unrelated"
    );
    assert_eq!(fs::read(root.join("notes.txt")).unwrap(), b"unrelated");
    assert!(other.join(provenance::target().1).exists());
    assert!(outcome.text.contains("notes.txt"), "{}", outcome.text);
}

#[test]
fn uninstall_failure_to_remove_marker_preserves_executable() {
    let directory = Directory::new();
    let (root, binary) = installation(&directory);
    fs::remove_file(root.join(MARKER)).unwrap();
    let error = uninstall(&root, &binary, None).err().unwrap();
    assert!(
        error.contains("marker") || error.contains(MARKER),
        "{error}"
    );
    assert_eq!(fs::read(&binary).unwrap(), b"old executable");
}
