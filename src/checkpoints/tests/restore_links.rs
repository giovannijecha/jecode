use super::*;
use std::path::Path;

#[test]
fn restore_preserves_a_literal_backslash_in_a_unix_link_target() {
    let fixture = Fixture::new();
    let target = r"literal\name.txt";
    fixture.write(target, b"original\n");
    fixture.write("literal/name.txt", b"decoy\n");
    let link = fixture.path("link");
    std::os::unix::fs::symlink(target, &link).unwrap();

    let workspace = fixture.open();
    let baseline = index(&workspace)
        .points
        .values()
        .find(|point| point.kind == "baseline")
        .unwrap()
        .seq;
    fs::remove_file(&link).unwrap();
    let selected = paths(&["link"]);
    let report = workspace
        .restore(&request(
            &workspace,
            baseline,
            Some(&selected),
            Expected::Latest,
        ))
        .unwrap();

    assert_eq!(restored(&report), ["link"]);
    assert_eq!(fs::read_link(&link).unwrap(), Path::new(target));
    assert_eq!(fs::read(&link).unwrap(), b"original\n");
    assert_eq!(fixture.read(target).unwrap(), b"original\n");
    assert_eq!(fixture.read("literal/name.txt").unwrap(), b"decoy\n");
}
