use super::*;

#[test]
fn model_search_matches_friendly_names_and_keeps_selection_in_the_filtered_list() {
    let models: Vec<_> = (0..12)
        .map(|index| Model {
            id: format!("provider/model-{index}"),
            name: format!("Friendly Model {index}"),
            prompt_price: None,
            completion_price: None,
            efforts: vec![Effort::Default],
        })
        .collect();
    let mut menu = Selector::models(&models, false, "provider/model-0");
    assert!(menu.searchable);
    for _ in 0..11 {
        menu.move_selection(false);
    }
    assert!(matches!(menu.chosen(), Some(Choice::Model(11))));
    menu.editor.insert("Friendly Model 9");
    menu.refresh();
    assert_eq!(menu.filtered.len(), 1);
    assert!(!menu.filtered[0].description.is_empty());
    assert!(matches!(menu.chosen(), Some(Choice::Model(9))));
    menu.editor.replace("no-such-model".into());
    menu.refresh();
    assert!(menu.chosen().is_none());
}
