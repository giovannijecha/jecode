use super::*;
use crate::attachments::{MARKER, Prompt};

fn message(role: &str, content: &str) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("content", Value::string(content)),
    ])
}

fn measure(context: &mut Context, source: &[Value], input: Option<usize>, output: Option<usize>) {
    let request = context.request_measurement(source, source, Inputs::default(), false, 100);
    context.record_request(
        Some(&Usage {
            input_tokens: input,
            output_tokens: output,
            ..Usage::default()
        }),
        request,
    );
}

#[test]
fn replayed_reasoning_is_removed_before_learning_text_growth() {
    let mut context = Context::default();
    let mut source = vec![message("system", "Agent"), message("user", "Inspect")];
    measure(&mut context, &source, Some(1000), Some(75_000));
    let first_bytes = context.measured_bytes.unwrap();
    let mut assistant = message("assistant", "Inspecting.");
    let assistant_bytes = portable(&assistant).encode().len();
    if let Value::Object(fields) = &mut assistant {
        fields.insert(
            "openai_items".into(),
            Value::Array(vec![Value::object([
                ("type", Value::string("reasoning")),
                ("encrypted_content", Value::string("opaque".repeat(20_000))),
            ])]),
        );
    }
    source.push(assistant);
    source.push(message("tool", &"source evidence ".repeat(500)));
    let snapshot = context.request_measurement(&source, &source, Inputs::default(), false, 100);
    let growth_bytes = snapshot.bytes - first_bytes - assistant_bytes;
    let growth_tokens = growth_bytes.div_ceil(6);
    context.record_request(
        Some(&Usage {
            input_tokens: Some(76_000 + growth_tokens),
            output_tokens: Some(10),
            ..Usage::default()
        }),
        snapshot,
    );
    assert_eq!(
        context.calibration,
        Some((1000 + growth_tokens, first_bytes + growth_bytes))
    );
    assert!(context.estimate_bytes(12_000) < 2_020);
    assert_eq!(context.input_tokens, Some(76_000 + growth_tokens));
}

#[test]
fn a_new_user_turn_without_replayed_reasoning_cannot_lower_text_density() {
    let mut context = Context::default();
    let mut source = vec![message("system", "Agent"), message("user", "Inspect")];
    measure(&mut context, &source, Some(1000), Some(3000));
    let density = context.calibration;
    source.push(message("assistant", "Done."));
    let pasted = "pasted log line ".repeat(1250);
    source.push(message("user", &pasted));
    // The provider dropped the 3,000 earlier reasoning tokens from this input.
    let input = 1000 + 2 + pasted.len() / 4;
    measure(&mut context, &source, Some(input), Some(20));
    assert_eq!(context.calibration, density);
    assert_eq!(context.input_tokens, Some(input));
}

#[test]
fn missing_usage_keeps_the_old_baseline_and_cannot_recalibrate_it() {
    let mut context = Context::default();
    let mut source = vec![message("system", "Agent"), message("user", "Inspect")];
    measure(&mut context, &source, Some(1000), Some(20));
    let original = context.value();
    source.push(message("assistant", "Reading"));
    source.push(message("tool", &"evidence ".repeat(1000)));
    measure(&mut context, &source, None, Some(10));
    assert_eq!(context.value(), original);
    source.push(message("assistant", "No usage was reported"));
    source.push(message("tool", "Next result"));
    let density = context.calibration;
    measure(&mut context, &source, Some(20_000), Some(30));
    assert_eq!(context.input_tokens, Some(20_000));
    assert_eq!(
        context.calibration, density,
        "A usage gap cannot become a text-density sample"
    );
}

#[test]
fn media_weights_are_tokens_at_every_text_density() {
    let mut image = crate::attachments::tests::attachment("att-1-1-1", "shot.png", "image/png");
    image.width = Some(4);
    image.height = Some(3);
    let source = vec![
        message("system", "Agent"),
        Prompt::new(MARKER.into(), vec![image]).message(),
    ];
    let mut context = Context::default();
    for (tokens, bytes) in [(1, 6), (1, 2), (1, 1)] {
        context.calibration = Some((tokens, bytes));
        let text = text_bytes(&source) + crate::tools::encoded_size_for(false);
        assert_eq!(
            context.estimate_projected_for(&source, Inputs::default(), false),
            context.estimate_bytes(text) + 85
        );
    }
}

#[test]
fn snapshot_measurements_roundtrip_and_old_sessions_remain_readable() {
    let mut context = Context::default();
    let source = vec![message("system", "Agent"), message("user", "Inspect")];
    measure(&mut context, &source, Some(1000), Some(20));
    context.output_message = Some(2);
    let value = context.value();
    assert_eq!(Context::parse(Some(&value)).unwrap().value(), value);
    let mut legacy = value;
    if let Value::Object(fields) = &mut legacy {
        for name in [
            "output_message",
            "measured_bytes",
            "measured_overhead_bytes",
        ] {
            fields.remove(name);
        }
    }
    let restored = Context::parse(Some(&legacy)).unwrap();
    assert_eq!(restored.input_tokens, Some(1000));
    assert_eq!(restored.measured_end, source.len());
    assert_eq!(restored.measured_bytes, None);
}

#[test]
fn legacy_whole_prompt_density_is_discarded_without_losing_usage() {
    let mut context = Context::default();
    measure(
        &mut context,
        &[message("system", "Agent")],
        Some(1000),
        Some(20),
    );
    let mut legacy = context.value();
    if let Value::Object(fields) = &mut legacy
        && let Some(Value::Object(calibration)) = fields.get_mut("token_calibration")
    {
        calibration.remove("method");
    }
    let restored = Context::parse(Some(&legacy)).unwrap();
    assert_eq!(restored.input_tokens, Some(1000));
    assert_eq!(restored.calibration, None);
}

#[test]
fn tiny_wrapper_changes_do_not_replace_a_substantial_text_sample() {
    let mut context = Context {
        calibration: Some((6000, 36_000)),
        ..Context::default()
    };
    context.learn_density((100, 100));
    assert_eq!(context.calibration, Some((6100, 36_100)));
    assert!(context.estimate_bytes(36_000) < 6100);
    context.calibration = Some((1_000_000, 6_000_000));
    context.learn_density((10_000, 60_000));
    assert_eq!(context.calibration.unwrap().1, 64 * 1024 + 60_000);
    assert!(context.estimate_bytes(12_000) <= 2001);
}
