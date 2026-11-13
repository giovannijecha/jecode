//! Attachments remain separate elements even beside incomplete quoted references.

use super::*;

#[test]
fn quoted_references_cannot_capture_attachment_markers() {
    let mut editor = Editor::default();
    typed(&mut editor, "See @\"docs/");
    editor.attach(image());
    editor.insert(" ");
    typed(&mut editor, "file.md\" ");
    let second = Attachment {
        name: "report.log".into(),
        media: "text/plain".into(),
        width: None,
        height: None,
        ..image()
    };
    editor.attach(second);

    assert_eq!(shown(&editor, 80), [" 1 Image ", " 2 report.log "]);
    assert!(references(&editor.text).is_empty());
    press(&mut editor, BACKSPACE, 0);
    assert_eq!(editor.attachments, [image()]);
    assert_eq!(shown(&editor, 80), [" 1 Image "]);
}
