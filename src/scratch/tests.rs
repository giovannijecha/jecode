use super::*;
use crate::{sessions::Store, test_support::Directory, tools::Tools};

fn area(home: &Directory, project: &Directory, id: &str) -> Area {
    Store::new(home.path().join("user data"), project.path())
        .unwrap()
        .temporary_area(id)
        .unwrap()
}

fn call(tools: &Tools, name: &str, entries: &[(&str, &str)]) -> Value {
    let arguments = Value::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).into(), Value::string(*value)))
            .collect(),
    );
    let result = tools.execute(name, &arguments.encode());
    assert!(result.get("error").is_none(), "{result:?}");
    result
}

#[test]
fn all_four_tools_share_the_session_area_without_changing_the_project_cwd() {
    let home = Directory::new();
    let project = Directory::new();
    fs::write(project.path().join("project.txt"), "project\n").unwrap();
    let temporary = area(&home, &project, "123-1-0");
    let root = temporary.ensure().unwrap();
    let mut tools = Tools::new(project.path()).unwrap();
    tools.configure_output(
        home.path().join("outputs"),
        crate::redact::Redactor::empty(),
    );
    tools.configure_temporary(temporary);
    call(
        &tools,
        "write",
        &[
            ("path", "tmp:probes/check.sh"),
            ("content", "printf 'before\\n'\n"),
        ],
    );
    call(
        &tools,
        "edit",
        &[
            ("path", "tmp:probes/check.sh"),
            ("old_text", "before"),
            ("new_text", "after"),
        ],
    );
    let read = call(&tools, "read", &[("path", "tmp:probes/check.sh")]);
    assert!(
        read.get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("after")
    );
    let output = call(
        &tools,
        "bash",
        &[(
            "command",
            concat!(
                "set -e; cat project.txt; bash \"$JECODE_TMP/probes/check.sh\"; ",
                "test -n \"$TMPDIR\"; test -n \"$TEMP\"; test -n \"$TMP\"; ",
                "created=$(mktemp \"$TMPDIR/check.XXXXXXXXXX\"); printf 'temp' > \"$created\"; ",
                "if command -v cygpath >/dev/null; then cygpath -m \"$created\"; else printf '%s\\n' \"$created\"; fi"
            ),
        )],
    );
    assert_eq!(
        output.get("exit_code"),
        Some(&Value::number(0)),
        "{output:?}"
    );
    let stdout = output.get("stdout").unwrap().as_str().unwrap();
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("project"));
    assert_eq!(lines.next(), Some("after"));
    let generated = fs::canonicalize(lines.next().unwrap()).unwrap();
    assert!(
        generated.starts_with(&root),
        "{generated:?} outside {root:?}"
    );
    assert_eq!(fs::read_to_string(generated).unwrap(), "temp");
    assert_eq!(fs::read_dir(project.path()).unwrap().count(), 1);
    let info = tools.temporary().unwrap().info().unwrap();
    assert_eq!(info.files, 2);
    assert_eq!(info.directories, 1);
    let absolute = environment_path(&root.join("probes/check.sh"))
        .to_string_lossy()
        .into_owned();
    assert!(
        call(&tools, "read", &[("path", &absolute)])
            .get("content")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("after")
    );
    #[cfg(windows)]
    {
        let msys = format!("/{}{}", absolute[..1].to_ascii_lowercase(), &absolute[2..]);
        assert!(
            call(&tools, "read", &[("path", &msys)])
                .get("content")
                .is_some()
        );
    }
}

#[cfg(windows)]
#[test]
fn native_windows_children_use_the_same_temporary_area() {
    let home = Directory::new();
    let project = Directory::new();
    let temporary = area(&home, &project, "123-2-0");
    let root = temporary.ensure().unwrap();
    let mut tools = Tools::new(project.path()).unwrap();
    tools.configure_output(
        home.path().join("outputs"),
        crate::redact::Redactor::empty(),
    );
    tools.configure_temporary(temporary);
    let output = call(
        &tools,
        "bash",
        &[(
            "command",
            "powershell.exe -NoProfile -NonInteractive -Command '[System.IO.Path]::GetTempFileName()'",
        )],
    );
    assert_eq!(
        output.get("exit_code"),
        Some(&Value::number(0)),
        "{output:?}"
    );
    let file = fs::canonicalize(output.get("stdout").unwrap().as_str().unwrap().trim()).unwrap();
    assert!(file.starts_with(root));
    assert_eq!(fs::read_dir(project.path()).unwrap().count(), 0);
}

#[test]
fn cleanup_is_scoped_and_keeps_identity_logs_and_other_sessions() {
    let home = Directory::new();
    let project = Directory::new();
    let first = area(&home, &project, "123-3-0");
    let other = area(&home, &project, "123-3-1");
    let (_, first_file) = first.file("nested/check.txt", true).unwrap();
    fs::write(&first_file, "scratch").unwrap();
    let (_, other_file) = other.file("keep.txt", true).unwrap();
    fs::write(&other_file, "other session").unwrap();
    fs::write(project.path().join("source.txt"), "project source").unwrap();
    let logs = Store::new(home.path().join("user data"), project.path())
        .unwrap()
        .output_directory();
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join("evidence.log"), "saved output").unwrap();
    let removed = first.clean().unwrap();
    assert_eq!(removed.files, 1);
    assert_eq!(removed.directories, 1);
    assert_eq!(removed.bytes, 7);
    assert!(!first_file.exists());
    assert_eq!(first.info().unwrap().files, 0);
    assert!(first.path().join(MARKER).is_file());
    assert_eq!(fs::read_to_string(other_file).unwrap(), "other session");
    assert_eq!(
        fs::read_to_string(logs.join("evidence.log")).unwrap(),
        "saved output"
    );
    assert!(project.path().join("source.txt").exists());
    let (_, again) = first.file("another.txt", true).unwrap();
    fs::write(again, "reused").unwrap();
    assert_eq!(first.info().unwrap().files, 1);
}

#[test]
fn invalid_aliases_and_foreign_ownership_preserve_existing_files() {
    let home = Directory::new();
    let project = Directory::new();
    let temporary = area(&home, &project, "123-4-0");
    let (_, path) = temporary.file("keep.txt", true).unwrap();
    fs::write(&path, "keep").unwrap();
    for alias in [
        "",
        "../outside",
        "/absolute",
        "C:/absolute",
        r"..\outside",
        MARKER,
        ".JECODE-TMP.JSON",
        "file:stream",
    ] {
        assert!(temporary.file(alias, true).is_err(), "{alias}");
    }
    let other = Directory::new();
    let wrong_project = Area::new(
        temporary.path().into(),
        fs::canonicalize(other.path()).unwrap(),
        "123-4-0",
    )
    .unwrap();
    assert!(wrong_project.clean().is_err());
    let wrong_session = Area::new(
        temporary.path().into(),
        fs::canonicalize(project.path()).unwrap(),
        "123-4-1",
    )
    .unwrap();
    assert!(wrong_session.clean().is_err());
    fs::write(temporary.path().join(MARKER), "damaged record").unwrap();
    assert!(temporary.clean().is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "keep");
}

#[test]
fn a_nonempty_unowned_directory_is_never_adopted_or_cleaned() {
    let home = Directory::new();
    let project = Directory::new();
    let temporary = area(&home, &project, "123-5-0");
    fs::create_dir_all(temporary.path()).unwrap();
    fs::write(temporary.path().join("foreign.txt"), "foreign").unwrap();
    assert!(temporary.clean().unwrap_err().contains("ownership"));
    assert!(!temporary.path().join(MARKER).exists());
    assert_eq!(
        fs::read_to_string(temporary.path().join("foreign.txt")).unwrap(),
        "foreign"
    );
}

#[test]
fn links_or_junctions_are_rejected_before_any_cleanup() {
    let home = Directory::new();
    let project = Directory::new();
    let outside = Directory::new();
    let temporary = area(&home, &project, "123-6-0");
    temporary.ensure().unwrap();
    fs::write(temporary.path().join("keep.txt"), "keep").unwrap();
    fs::write(outside.path().join("outside.txt"), "outside").unwrap();
    let link = temporary.path().join("linked");
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
    assert!(temporary.file("linked/outside.txt", true).is_err());
    assert!(temporary.file("linked/outside.txt", false).is_err());
    assert!(temporary.clean().unwrap_err().contains("link"));
    assert_eq!(
        fs::read_to_string(temporary.path().join("keep.txt")).unwrap(),
        "keep"
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("outside.txt")).unwrap(),
        "outside"
    );
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
    temporary.clean().unwrap();
}
