use super::*;
use crate::{
    attachments::{MARKER, Prompt, tests::png},
    json::{self, Value},
    test_support::Directory,
};

const PDF: &[u8] = b"%PDF-1.4\n1 0 obj << /Type /Page >> endobj\n%%EOF\n";

fn texts(message: &Value) -> String {
    message
        .get("content")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}

fn types(message: &Value) -> Vec<String> {
    message
        .get("content")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|part| part.get("type").and_then(Value::as_str).unwrap().to_owned())
        .collect()
}

fn known(image: bool, file: bool) -> Inputs {
    Inputs {
        image: Some(image),
        file: Some(file),
    }
}

#[test]
fn user_attachments_become_content_parts_with_a_manifest() {
    let root = Directory::new();
    let pool = Pool::new(root.path().to_path_buf());
    let image = pool.import_bytes("shot.png", &png(4, 3)).unwrap();
    let pdf = pool.import_bytes("report.pdf", PDF).unwrap();
    let text = pool.import_bytes("notes.txt", b"hello").unwrap();
    let binary = pool.import_bytes("blob.bin", &[0, 159, 146, 150]).unwrap();
    let prompt = Prompt::new(
        format!("compare {MARKER} {MARKER} {MARKER} {MARKER}"),
        vec![image.clone(), pdf.clone(), text.clone(), binary.clone()],
    );
    let system = Value::object([
        ("role", Value::string("system")),
        ("content", Value::string("s")),
    ]);
    let messages = vec![system.clone(), prompt.message()];
    let request = materialize(messages.clone(), Some(&pool), known(true, false)).unwrap();
    assert_eq!(request[0], system);
    let user = &request[1];
    assert!(user.get("attachments").is_none());
    assert_eq!(types(user), ["text", "image_url", "file"]);
    let manifest = texts(user);
    assert!(manifest.starts_with("compare [1# Image] [2# File: report.pdf]"));
    assert!(manifest.contains(&format!("reference {}", image.reference())));
    assert!(manifest.contains("image included"));
    assert!(manifest.contains("PDF included; OpenRouter file-parser engine: cloudflare-ai"));
    assert!(manifest.contains(&format!(
        "text not inlined; read it with the read tool, path {}",
        text.reference()
    )));
    assert!(manifest.contains("blob.bin"));
    assert!(manifest.contains("not interpreted; only the original bytes are available locally"));
    assert!(manifest.contains("local copy: "));
    let url = user.get("content").and_then(Value::as_array).unwrap()[1]
        .get("image_url")
        .and_then(|image| image.get("url"))
        .and_then(Value::as_str)
        .unwrap()
        .to_owned();
    assert!(url.starts_with("data:image/png;base64,"));
    assert_eq!(
        crate::attachments::base64::decode(&url["data:image/png;base64,".len()..]).unwrap(),
        png(4, 3)
    );
    assert!(has_files(&request));
    assert!(!has_files(&messages));
    // The stored history keeps text and metadata only.
    assert!(messages[1].get("content").and_then(Value::as_str).is_some());
}

#[test]
fn pdf_inline_limit_keeps_oversized_files_local() {
    let root = Directory::new();
    let pool = Pool::new(root.path().to_path_buf());
    let mut pdf = pool.import_bytes("report.pdf", PDF).unwrap();
    pdf.size = PDF_INLINE_BYTES;
    let message = |pdf: &Attachment| Prompt::new(MARKER.to_string(), vec![pdf.clone()]).message();
    let included = materialize(vec![message(&pdf)], Some(&pool), Inputs::default()).unwrap();
    assert_eq!(types(&included[0]), ["text", "file"]);
    pdf.size += 1;
    let local = materialize(vec![message(&pdf)], Some(&pool), Inputs::default()).unwrap();
    assert_eq!(types(&local[0]), ["text"]);
    assert!(texts(&local[0]).contains("not sent: larger than 32.0 MiB"));
    assert!(texts(&local[0]).contains(&pdf.reference()));
}

#[test]
fn model_capabilities_choose_parts_and_the_pdf_engine() {
    let root = Directory::new();
    let pool = Pool::new(root.path().to_path_buf());
    let image = pool.import_bytes("shot.png", &png(2, 2)).unwrap();
    let message = Prompt::new(MARKER.to_string(), vec![image]).message();
    let request = materialize(vec![message.clone()], Some(&pool), known(false, true)).unwrap();
    assert_eq!(types(&request[0]), ["text"]);
    let manifest = texts(&request[0]);
    // An attachment-only message keeps its label beside the manifest.
    assert!(manifest.starts_with("[1# Image]\n\nAttachments:\n[1# Image] shot.png"));
    assert!(manifest.contains("not sent: the selected model does not accept image input"));
    // Unknown capabilities still send the image.
    let unknown = materialize(vec![message.clone()], Some(&pool), Inputs::default()).unwrap();
    assert_eq!(types(&unknown[0]), ["text", "image_url"]);
    let missing = materialize(vec![message], None, Inputs::default()).unwrap();
    assert!(texts(&missing[0]).contains("attachment storage is unavailable"));
    assert_eq!(engine(Some(true)), "native");
    assert_eq!(engine(None), "cloudflare-ai");
    assert_eq!(
        plugins(Some(true)).encode(),
        r#"[{"id":"file-parser","pdf":{"engine":"native"}}]"#
    );
}

#[test]
fn read_views_follow_their_tool_results() {
    let root = Directory::new();
    let pool = Pool::new(root.path().to_path_buf());
    let image = pool.import_bytes("shot.png", &png(4, 3)).unwrap();
    let result = Value::object([
        ("view", Value::string("image")),
        ("attachment", image.value()),
    ]);
    let tool = |id: &str, content: String| {
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            ("content", Value::string(content)),
        ])
    };
    let messages = vec![
        tool("a", result.encode()),
        tool("b", "{\"content\":\"plain\"}".into()),
    ];
    let text_only = materialize(messages.clone(), Some(&pool), known(false, false)).unwrap();
    assert_eq!(types(&text_only[2]), ["text"]);
    assert_eq!(weight_for(&messages[0], known(false, false)), 0);
    let request = materialize(messages, Some(&pool), Inputs::default()).unwrap();
    // Tool results stay contiguous; the view follows the run.
    let roles = request
        .iter()
        .map(|message| message.get("role").and_then(Value::as_str).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(roles, ["tool", "tool", "user"]);
    assert_eq!(types(&request[2]), ["text", "image_url"]);
    assert!(texts(&request[2]).contains(&format!("{} follows.", image.reference())));
    assert_eq!(weight(&tool("a", result.encode())), 85);
    assert_eq!(weight(&tool("b", "{}".into())), 0);
}

#[test]
fn read_views_are_sent_once_and_can_be_requested_again() {
    let root = Directory::new();
    let pool = Pool::new(root.path().to_path_buf());
    let first = pool.import_bytes("first.png", &png(4, 3)).unwrap();
    let second = pool.import_bytes("second.png", &png(5, 2)).unwrap();
    let tool = |id: &str, attachment: &Attachment| {
        Value::object([
            ("role", Value::string("tool")),
            ("tool_call_id", Value::string(id)),
            (
                "content",
                Value::string(
                    Value::object([
                        ("view", Value::string("image")),
                        ("attachment", attachment.value()),
                    ])
                    .encode(),
                ),
            ),
        ])
    };
    let first_result = tool("a", &first);
    let second_result = tool("b", &second);
    let after_tools = vec![first_result.clone(), second_result.clone()];
    let current = materialize(after_tools.clone(), Some(&pool), known(true, false)).unwrap();
    assert_eq!(
        materialize(after_tools.clone(), Some(&pool), known(true, false)).unwrap(),
        current,
        "a retry before the assistant response must keep the requested views"
    );
    assert_eq!(types(&current[2]), ["text", "image_url", "image_url"]);
    let first_url = current[2].get("content").and_then(Value::as_array).unwrap()[1]
        .get("image_url")
        .and_then(|part| part.get("url"))
        .and_then(Value::as_str)
        .unwrap();

    let user = Value::object([
        ("role", Value::string("user")),
        ("content", Value::string("Please compare both images")),
    ]);
    let with_user = materialize(
        vec![first_result.clone(), second_result.clone(), user.clone()],
        Some(&pool),
        known(true, false),
    )
    .unwrap();
    assert_eq!(types(&with_user[2]), ["text", "image_url", "image_url"]);
    assert_eq!(with_user[3], user);

    let answer = Value::object([
        ("role", Value::string("assistant")),
        ("content", Value::string("seen")),
        ("reasoning_details", Value::string("opaque provider data")),
    ]);
    let history = vec![first_result.clone(), second_result, answer.clone()];
    let later = materialize(history.clone(), Some(&pool), known(true, false)).unwrap();
    assert_eq!(later, history);

    let reread = materialize(
        vec![first_result, answer, tool("c", &first)],
        Some(&pool),
        known(true, false),
    )
    .unwrap();
    assert_eq!(
        reread[1].get("reasoning_details"),
        Some(&Value::string("opaque provider data"))
    );
    assert_eq!(types(&reread[3]), ["text", "image_url"]);
    let reread_url = reread[3].get("content").and_then(Value::as_array).unwrap()[1]
        .get("image_url")
        .and_then(|part| part.get("url"))
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(reread_url, first_url);
}

#[test]
fn weights_follow_pixels_and_pages_not_payload_size() {
    let mut image = crate::attachments::tests::attachment("att-1-1-1", "a.png", "image/png");
    image.size = 40_000_000;
    let message = |attachment: &Attachment| {
        Prompt::new(MARKER.to_string(), vec![attachment.clone()]).message()
    };
    assert_eq!(weight(&message(&image)), 1600);
    image.width = Some(10);
    image.height = Some(10);
    assert_eq!(weight(&message(&image)), 85);
    image.width = Some(4000);
    image.height = Some(1000);
    assert_eq!(weight(&message(&image)), 1398);
    let mut pdf = crate::attachments::tests::attachment("att-1-1-2", "a.pdf", "application/pdf");
    pdf.pages = Some(3);
    assert_eq!(weight(&message(&pdf)), 4500);
    let binary =
        crate::attachments::tests::attachment("att-1-1-3", "a.bin", "application/octet-stream");
    assert_eq!(weight(&message(&binary)), 0);
    assert_eq!(
        weight(&Value::object([("role", Value::string("assistant"))])),
        0
    );
}

#[test]
fn text_views_list_references() {
    let pdf = crate::attachments::tests::attachment("att-1-1-2", "a.pdf", "application/pdf");
    let message = Prompt::new(format!("read {MARKER}"), vec![pdf]).message();
    let text = user_text(&message);
    assert!(text.starts_with("read [1# File: a.pdf]\n\nAttachments:\n[1# File: a.pdf] a.pdf ("));
    assert!(text.ends_with("): attachment:att-1-1-2"));
    let plain = json::parse(r#"{"role":"user","content":"hi"}"#).unwrap();
    assert_eq!(user_text(&plain), "hi");
}
