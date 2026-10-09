use super::Agent;
use crate::json::Value;

impl Agent {
    pub(super) fn record_report_delivery(&self) {
        let references = {
            let messages = self.messages.lock().unwrap();
            messages
                .iter()
                .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
                .map(|request| (request, messages.len() - 1))
        };
        if let Some((request, response)) = references {
            self.events.lock().unwrap().push(self.redactor.value(&Value::object([
                ("type", Value::string("local_command")),
                ("kind", Value::string("notice")),
                ("receipt", Value::string("report_delivery")),
                ("command", Value::string("Report delivery")),
                ("result", Value::string("Final response delivered after native completion guards; its contents are not independent verification proof")),
                ("details", Value::Array(vec![
                    Value::Array(vec![Value::string("request_history"), Value::string(request.to_string())]),
                    Value::Array(vec![Value::string("response_history"), Value::string(response.to_string())]),
                ])),
                ("after_message", Value::number(response + 1)),
                ("model", Value::string(self.model())),
                ("effort", Value::string(self.effort().name())),
            ])));
        }
    }

    pub(super) fn delivered_report_hint(&self) -> String {
        // Release the event lock before reading messages: archive readers and
        // compaction must never acquire these two locks in opposite order.
        let receipt = self.events.lock().unwrap().iter().rev().find_map(|event| {
            if event.get("command").and_then(Value::as_str) != Some("Report delivery")
                || event.get("receipt").and_then(Value::as_str) != Some("report_delivery")
            {
                return None;
            }
            let details = event.get("details")?.as_array()?;
            let reference = |key: &str| {
                details.iter().find_map(|pair| {
                    let pair = pair.as_array()?;
                    (pair.first()?.as_str()? == key)
                        .then(|| pair.get(1)?.as_str()?.parse::<usize>().ok())
                        .flatten()
                })
            };
            Some((
                reference("request_history")?,
                reference("response_history")?,
            ))
        });
        let Some((request, response)) = receipt else {
            return String::new();
        };
        let messages = self.messages.lock().unwrap();
        if request >= response
            || messages
                .get(request)
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                != Some("user")
            || !messages.get(response).is_some_and(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message
                        .get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty())
                    && message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_none_or(|calls| calls.is_empty())
            })
        {
            return String::new();
        }
        format!(
            "Native report delivery: the final response at history:{response} was delivered for user request history:{request}. Earlier reporting work for that request has been delivered; later user requests can require another response. This receipt proves delivery only, not correctness, passed checks or acceptance coverage.\n\n"
        )
    }
}
