use super::super::transport::Options;
use crate::json::Value;

pub(super) fn parse(arguments: &Value) -> Result<Options, String> {
    let options = Options::from(arguments)?;
    if options.method != "GET" || options.body.is_some() {
        return Err(
            "crawl uses GET without a body; use fetch or download for other methods".into(),
        );
    }
    Ok(options)
}

pub(super) fn encode(options: &Options) -> Value {
    let mut value = Value::object([
        (
            "headers",
            Value::Object(
                options
                    .headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Value::string(value)))
                    .collect(),
            ),
        ),
        ("timeout_seconds", Value::number(options.timeout_seconds)),
    ]);
    if let Some(limit) = options.max_bytes
        && let Value::Object(fields) = &mut value
    {
        fields.insert("max_bytes".into(), Value::number(limit));
    }
    value
}

pub(super) fn resume(arguments: &Value, saved: &Options) -> Result<(), String> {
    let supplied = encode(&parse(arguments)?);
    let saved = encode(saved);
    for key in ["headers", "timeout_seconds", "max_bytes"] {
        if arguments.get(key).is_some() && supplied.get(key) != saved.get(key) {
            return Err(format!(
                "{key} differs from the saved crawl options; start a new crawl to change it"
            ));
        }
    }
    Ok(())
}
