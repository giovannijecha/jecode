use super::*;
use crate::config::Settings;
use crate::test_support::Directory;
use std::fs;

#[test]
fn skill_listing_and_source_mask_saved_credentials_in_text_and_json() {
    let directory = Directory::new();
    let context = Context::at(directory.path());
    let key = "isolated-openrouter-secret-in-skill";
    context
        .store
        .save(&Settings::new(key.into(), "fixture/model".into()).unwrap())
        .unwrap();
    let package = directory.path().join("skills/check");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        format!("---\nname: check\ndescription: Verify {key}\n---\n\nUse {key} here.\n"),
    )
    .unwrap();
    for (action, operands) in [("list", &[][..]), ("show", &["check"][..])] {
        let command = Manage {
            family: "skills",
            action,
            operands: operands.iter().map(|value| (*value).into()).collect(),
            values: vec![],
        };
        let report = run(&context, &command).unwrap();
        assert!(!report.text.contains(key), "{}", report.text);
        assert!(
            !report.value.pretty().contains(key),
            "{}",
            report.value.pretty()
        );
        assert!(report.text.contains("[redacted]"));
    }
}
