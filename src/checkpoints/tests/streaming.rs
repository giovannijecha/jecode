//! Live files can grow after discovery; capture and undo must stay bounded.
use super::*;

#[test]
fn grown_file_batches_restore_and_undo_every_byte() {
    let fixture = Fixture::new();
    let names: Vec<_> = (0..24)
        .map(|index| format!("files/f{index:02}.bin"))
        .collect();
    for name in &names {
        fixture.write(name, b"small baseline\n");
    }
    let workspace = fixture.open();
    let target = workspace
        .mark("turn", "small", "test", false, false)
        .unwrap();
    for (index, name) in names.iter().enumerate() {
        fixture.write(name, &vec![index as u8; 512 << 10]);
    }
    // Both the catch-up scan and restore must consume multiple live batches.
    let report = workspace
        .restore(&request(&workspace, target, None, Expected::Latest))
        .unwrap();
    assert!(report.failed.is_none());
    assert!(report.conflicts.is_empty());
    assert_eq!(report.applied.len(), names.len());
    for name in &names {
        assert_eq!(fixture.read(name).unwrap(), b"small baseline\n");
    }
    let undone = workspace
        .restore(&request(
            &workspace,
            report.before.unwrap(),
            None,
            Expected::Latest,
        ))
        .unwrap();
    assert!(undone.failed.is_none());
    assert!(undone.conflicts.is_empty());
    assert_eq!(undone.applied.len(), names.len());
    for (index, name) in names.iter().enumerate() {
        let bytes = fixture.read(name).unwrap();
        assert_eq!(bytes.len(), 512 << 10);
        assert!(bytes.iter().all(|byte| *byte == index as u8));
    }
}
