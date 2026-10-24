use super::*;
use crate::attachments::{MARKER, Prompt};
use crate::backend::Usage;
use crate::context::{self, portable};
use crate::test_support::Directory;

fn agent(directory: &Directory) -> Agent {
    Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    )
}

fn message(role: &str, content: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(content)),
    ])
}

fn image_receipt(image: &crate::attachments::Attachment) -> Value {
    let receipt = Value::object([
        ("view", Value::string("image")),
        ("attachment", image.value()),
    ]);
    message("tool", &receipt.encode())
}

fn image() -> crate::attachments::Attachment {
    let mut image = crate::attachments::tests::attachment("att-1-1-1", "shot.png", "image/png");
    image.width = Some(4);
    image.height = Some(3);
    image
}

#[test]
fn measured_history_uses_calibrated_growth_and_falls_back_without_density() {
    let directory = Directory::new();
    let mut agent = agent(&directory);
    agent
        .messages
        .lock()
        .unwrap()
        .push(message("user", "Inspect"));
    agent.context.observe(
        Some(&Usage {
            input_tokens: Some(1000),
            ..Usage::default()
        }),
        2,
    );
    agent.context.calibrate(6000);
    let appended = message("tool", &"new output ".repeat(1000));
    let size = appended.encode().len();
    agent.messages.lock().unwrap().push(appended);
    assert_eq!(agent.context_status().tokens, 1000 + size.div_ceil(2));
    agent.context.calibration = None;
    assert_eq!(agent.context_status().tokens, 1000 + size);
}

#[test]
fn tool_views_expire_from_context_budget_after_an_assistant_response() {
    let directory = Directory::new();
    let mut agent = agent(&directory);
    let image = image();
    let user = Prompt::new(MARKER.to_string(), vec![image.clone()]).message();
    let tool = image_receipt(&image);
    {
        let mut messages = agent.messages.lock().unwrap();
        messages.extend([user, tool]);
        let text_bytes = messages
            .iter()
            .map(|item| portable(item).encode().len())
            .sum::<usize>();
        assert_eq!(
            context::bytes_for(&messages, agent.client.inputs()),
            text_bytes + 170
        );
    }
    agent.context.observe(
        Some(&Usage {
            input_tokens: Some(500),
            ..Usage::default()
        }),
        3,
    );
    agent.context.calibrate(1000);
    agent
        .messages
        .lock()
        .unwrap()
        .push(message("user", "Compare the image"));
    assert!(agent.context_status().tokens >= 500);

    agent
        .messages
        .lock()
        .unwrap()
        .push(message("assistant", "I saw it"));
    let messages = agent.messages.lock().unwrap();
    let text_bytes = messages
        .iter()
        .map(|item| portable(item).encode().len())
        .sum::<usize>();
    assert_eq!(
        context::bytes_for(&messages, agent.client.inputs()),
        text_bytes + 85
    );
    assert_eq!(agent.context.measured_input(&messages), None);
    let projected = agent.projected_context(&agent.context, &messages);
    let expected = agent.context.estimate_bytes(
        context::bytes_for(&projected, agent.client.inputs()) + crate::tools::encoded_size(),
    ) + agent.request_environment().len();
    drop(messages);
    assert_eq!(agent.context_status().tokens, expected);
}

#[test]
fn measured_growth_charges_only_active_tool_views_and_keeps_user_media() {
    let directory = Directory::new();
    let mut agent = agent(&directory);
    agent
        .messages
        .lock()
        .unwrap()
        .push(message("user", "Inspect"));
    agent.context.observe(
        Some(&Usage {
            input_tokens: Some(500),
            ..Usage::default()
        }),
        2,
    );
    let image = image();
    let tool = image_receipt(&image);
    let appended = [
        tool.clone(),
        message("assistant", "Viewed"),
        Prompt::new(MARKER.to_string(), vec![image]).message(),
        tool,
        message("user", "Compare both images"),
    ];
    agent.messages.lock().unwrap().extend(appended);
    let messages = agent.messages.lock().unwrap();
    let recent = &messages[agent.context.measured_end..];
    let text_bytes = recent
        .iter()
        .map(|item| portable(item).encode().len())
        .sum::<usize>();
    let isolated_bytes = recent
        .iter()
        .map(|item| context::bytes_for(std::slice::from_ref(item), agent.client.inputs()))
        .sum::<usize>();
    assert_eq!(isolated_bytes, text_bytes + 85 * 3);
    drop(messages);
    // The earlier tool view expired; the user image and latest tool view remain.
    assert_eq!(agent.context_status().tokens, 500 + text_bytes + 85 * 2);
}

#[test]
fn returned_messages_do_not_enter_the_unmeasured_agent_estimate() {
    let directory = Directory::new();
    let mut agent = agent(&directory);
    let image = image();
    agent.messages.lock().unwrap().extend([
        message("user", "Original task"),
        Prompt::new("Failed prompt".into(), vec![image]).message(),
        message("user", "Additional unanswered input"),
        message("user", "Continue without the image"),
    ]);
    let messages = agent.messages.lock().unwrap().clone();
    let original = agent.context_status().tokens;
    let event = Value::object([
        ("type", Value::string("turn_returned")),
        ("from_message", Value::number(2)),
        ("after_message", Value::number(4)),
    ]);
    agent.context.restore_returned(&[event], messages.len());
    let projected = agent.projected_context(&agent.context, &messages);
    assert_eq!(
        projected,
        vec![
            messages[0].clone(),
            messages[1].clone(),
            messages[4].clone()
        ]
    );
    let expected = context::bytes_for(&projected, agent.client.inputs())
        + crate::tools::encoded_size()
        + agent.request_environment().len();
    assert_eq!(agent.context_status().tokens, expected);
    assert!(expected < original);
}
