use super::*;
use crate::test_support::Directory;

const PDF: &[u8] = b"%PDF-1.4\n1 0 obj << /Type /Page >> endobj\n%%EOF\n";

fn progressive_jpeg() -> Vec<u8> {
    // SOF2 reports 30x20; media classification depends on these bytes, not the name.
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0];
    bytes.extend([0xff, 0xc2, 0, 11, 8, 0, 20, 0, 30, 3, 0, 0]);
    bytes
}

fn read(tools: &Tools, path: &str) -> Value {
    tools.execute(
        "read",
        &Value::object([("path", Value::string(path))]).encode(),
    )
}

#[test]
fn local_image_and_pdf_reads_import_content_by_signature() {
    let project = Directory::new();
    let home = Directory::new();
    let mut tools = Tools::new(project.path()).unwrap();
    let pool = crate::attachments::Pool::new(home.path().join("attachments"));
    tools.configure_attachments(pool.clone());

    for (name, bytes, view, media) in [
        (
            "photo.bin",
            crate::attachments::tests::png(2, 3),
            "image",
            "image/png",
        ),
        ("progressive.bin", progressive_jpeg(), "image", "image/jpeg"),
        ("report.bin", PDF.to_vec(), "pdf", "application/pdf"),
    ] {
        fs::write(project.path().join(name), &bytes).unwrap();
        let shown = read(&tools, name);
        assert_eq!(
            shown.get("view").and_then(Value::as_str),
            Some(view),
            "{shown:?}"
        );
        assert_eq!(
            shown
                .get("attachment")
                .and_then(|entry| entry.get("media"))
                .and_then(Value::as_str),
            Some(media),
        );
        assert!(!shown.encode().contains("base64,"));
        let reference = shown.get("reference").and_then(Value::as_str).unwrap();
        let again = read(&tools, reference);
        assert_eq!(again.get("view").and_then(Value::as_str), Some(view));
        let id = reference.strip_prefix("attachment:").unwrap();
        assert_eq!(fs::read(pool.load(id).unwrap().path).unwrap(), bytes);
    }
}

#[test]
fn temporary_media_reads_accept_alias_and_native_path() {
    let project = Directory::new();
    let home = Directory::new();
    let mut tools = Tools::new(project.path()).unwrap();
    let store = crate::sessions::Store::new(home.path().to_path_buf(), project.path()).unwrap();
    let area = store.temporary_area("123-1-0").unwrap();
    let temp = area.ensure().unwrap();
    tools.configure_temporary(area);
    let pool = store.attachments();
    tools.configure_attachments(pool.clone());

    let image = crate::attachments::tests::png(4, 2);
    fs::write(temp.join("camera.bin"), &image).unwrap();
    fs::write(temp.join("report.bin"), PDF).unwrap();
    for (path, bytes, view) in [
        ("tmp:camera.bin".to_string(), image.as_slice(), "image"),
        (
            crate::scratch::environment_path(&temp.join("report.bin"))
                .to_string_lossy()
                .into_owned(),
            PDF,
            "pdf",
        ),
    ] {
        let shown = read(&tools, &path);
        assert_eq!(
            shown.get("view").and_then(Value::as_str),
            Some(view),
            "{shown:?}"
        );
        let id = shown
            .get("reference")
            .and_then(Value::as_str)
            .unwrap()
            .strip_prefix("attachment:")
            .unwrap();
        assert_eq!(fs::read(pool.load(id).unwrap().path).unwrap(), bytes);
    }
}

#[test]
fn media_names_do_not_override_signatures_or_text_paging() {
    let project = Directory::new();
    let home = Directory::new();
    let mut tools = Tools::new(project.path()).unwrap();
    let pool = crate::attachments::Pool::new(home.path().join("attachments"));
    tools.configure_attachments(pool);
    fs::write(
        project.path().join("looks-like-image.jpg"),
        "first\nsecond\nthird",
    )
    .unwrap();
    let page = tools.execute(
        "read",
        &Value::object([
            ("path", Value::string("looks-like-image.jpg")),
            ("offset", Value::number(2)),
            ("limit", Value::number(1)),
        ])
        .encode(),
    );
    assert_eq!(
        page.get("content").and_then(Value::as_str),
        Some("2: second\n")
    );
    assert_eq!(page.get("next_offset").and_then(Value::as_usize), Some(3));
    assert!(page.get("attachment").is_none());
    fs::write(project.path().join("looks-like-pdf.pdf"), "plain text").unwrap();
    assert!(read(&tools, "looks-like-pdf.pdf").get("content").is_some());
}

#[test]
fn media_read_without_attachment_storage_reports_the_missing_dependency() {
    let project = Directory::new();
    let tools = Tools::new(project.path()).unwrap();
    fs::write(
        project.path().join("photo.png"),
        crate::attachments::tests::png(2, 2),
    )
    .unwrap();
    fs::write(project.path().join("notes.txt"), "still readable").unwrap();
    let error = read(&tools, "photo.png")
        .get("error")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    assert!(
        error.contains("require session attachment storage"),
        "{error}"
    );
    assert!(read(&tools, "notes.txt").get("content").is_some());
}

#[test]
fn media_import_cannot_expand_the_read_file_scope() {
    let project = Directory::new();
    let outside = Directory::new();
    let home = Directory::new();
    let mut tools = Tools::new(project.path()).unwrap();
    let pool = crate::attachments::Pool::new(home.path().join("attachments"));
    tools.configure_attachments(pool.clone());
    fs::write(
        outside.path().join("secret.png"),
        crate::attachments::tests::png(2, 2),
    )
    .unwrap();
    for path in [
        "../secret.png".to_string(),
        outside
            .path()
            .join("secret.png")
            .to_string_lossy()
            .into_owned(),
    ] {
        assert!(read(&tools, &path).get("error").is_some(), "{path}");
    }
    assert!(!pool.directory().exists());
}

#[cfg(any(unix, windows))]
#[test]
fn media_import_rejects_link_outside_project() {
    let project = Directory::new();
    let outside = Directory::new();
    let home = Directory::new();
    let mut tools = Tools::new(project.path()).unwrap();
    tools.configure_attachments(crate::attachments::Pool::new(
        home.path().join("attachments"),
    ));
    let target = outside.path().join("secret.bin");
    fs::write(&target, crate::attachments::tests::png(2, 2)).unwrap();
    let link = project.path().join("linked");
    #[cfg(windows)]
    {
        let created = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(outside.path().to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    assert!(read(&tools, "linked/secret.bin").get("error").is_some());
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
}
