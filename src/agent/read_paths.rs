//! One read call for several paths. Models tend to send one call per
//! response, and every response sends the conversation again; reading the
//! files a step needs together saves those round trips.

use super::Agent;
use crate::{backend::ToolCall, json::Value, output, tools::Begun};

/// Inline size of one call's results. Every path is still read: entries past
/// this budget are saved in call order to a result file the model pages with
/// offset and limit, so a large batch never drops a path.
const INLINE: usize = 96 * 1024;

impl Agent {
    /// Reads each path with its default window, in order, through the same
    /// route as a single read. Every entry reports its own result or error,
    /// so one missing file needs no retry of the others.
    pub(super) fn read_paths(&self, call: &ToolCall, arguments: &Value, begun: &Begun) -> Value {
        self.read_paths_with(call, arguments, begun, |first| {
            Overflow::start(self.tools.outputs(), first)
        })
    }

    fn read_paths_with(
        &self,
        call: &ToolCall,
        arguments: &Value,
        begun: &Begun,
        mut save: impl FnMut(usize) -> Result<Overflow, String>,
    ) -> Value {
        let Some(paths) = arguments.get("paths").and_then(Value::as_array) else {
            return failure("paths must be an array of strings");
        };
        if arguments.get("path").is_some() {
            return failure("Use path or paths, not both");
        }
        if paths.is_empty() {
            return failure("paths must contain at least one path");
        }
        let mut inline = Vec::new();
        let mut used = 0usize;
        let mut overflow: Option<Overflow> = None;
        let (mut read, mut not_read, mut lines) = (0usize, 0usize, 0usize);
        let mut skills = std::collections::BTreeSet::new();
        for (index, path) in paths.iter().enumerate() {
            let result = match path.as_str() {
                Some(path) => entry(path, self.read_one(call, path, begun.read_member(index))),
                None => failure("paths must contain strings"),
            };
            if result.get("error").is_some() {
                not_read += 1;
            } else {
                read += 1;
            }
            lines = lines.saturating_add(
                result
                    .get("lines_returned")
                    .and_then(Value::as_usize)
                    .unwrap_or(0),
            );
            if let Some(reference) = result.get("skill_reference").and_then(Value::as_str) {
                skills.insert(reference.to_owned());
            }
            let size = result.encode().len();
            if overflow.is_none() && used.saturating_add(size) <= INLINE {
                used += size;
                inline.push(result);
                continue;
            }
            if overflow.is_none() {
                match save(inline.len()) {
                    Ok(started) => overflow = Some(started),
                    // Without a result file the entry stays inline; the
                    // context view still keeps it recoverable from history.
                    Err(_) => {
                        inline.push(result);
                        continue;
                    }
                }
            }
            if let Some(file) = &mut overflow
                && let Err(error) = file.push(&result)
            {
                inline.push(result);
                return batch_result(
                    inline,
                    (read, not_read, lines),
                    skills,
                    Some(file.saved()),
                    Some(ReadFailure {
                        error,
                        processed: index + 1,
                        total: paths.len(),
                    }),
                );
            }
        }
        let mut saving_failure = None;
        let saved = overflow.as_mut().map(|file| {
            if let Err(error) = file.finish() {
                saving_failure = Some(ReadFailure {
                    error,
                    processed: paths.len(),
                    total: paths.len(),
                });
            }
            file.saved()
        });
        batch_result(
            inline,
            (read, not_read, lines),
            skills,
            saved,
            saving_failure,
        )
    }

    fn read_one(&self, call: &ToolCall, path: &str, begun: Begun) -> Value {
        let single = Value::object([("path", Value::string(path))]);
        let read = ToolCall {
            id: call.id.clone(),
            name: "read".into(),
            arguments: single.encode(),
        };
        shown_alone(self.execute_tool_with_change(&read, begun).result)
    }
}

/// Entries past the inline budget, written as they are read so memory stays
/// bounded however many paths the call names.
struct Overflow {
    logs: output::Logs,
    first: usize,
    count: usize,
}

struct Saved {
    reference: String,
    first: usize,
    count: usize,
}

struct ReadFailure {
    error: String,
    processed: usize,
    total: usize,
}

fn batch_result(
    files: Vec<Value>,
    (read, not_read, lines): (usize, usize, usize),
    skills: std::collections::BTreeSet<String>,
    saved: Option<Saved>,
    failure: Option<ReadFailure>,
) -> Value {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("files".into(), Value::Array(files));
    let has_saved = saved.as_ref().is_some_and(|saved| saved.count > 0);
    if let Some(saved) = saved.filter(|saved| saved.count > 0) {
        saved.describe(&mut fields);
    }
    if has_saved || failure.is_some() {
        fields.insert("files_read".into(), Value::number(read));
        fields.insert("files_not_read".into(), Value::number(not_read));
        fields.insert("lines_read".into(), Value::number(lines));
        if !skills.is_empty() {
            fields.insert(
                "skill_references".into(),
                Value::Array(skills.into_iter().map(Value::string).collect()),
            );
        }
    }
    if let Some(ReadFailure {
        error,
        processed,
        total,
    }) = failure
    {
        fields.insert("error".into(), Value::string(error));
        fields.insert("paths_processed".into(), Value::number(processed));
        fields.insert(
            "next_path_index".into(),
            if processed < total {
                Value::number(processed)
            } else {
                Value::Null
            },
        );
        if has_saved {
            fields.insert("result_file_incomplete".into(), Value::Bool(true));
        }
        fields.insert("notice".into(), Value::string(format!(
            "Processed {processed} of {total} paths before saving failed. files retains the results still available inline. Any result_file may end with a partial entry. Resume remaining reads at paths[next_path_index] when non-null."
        )));
    }
    Value::Object(fields)
}

impl Overflow {
    fn start(outputs: &output::Store, first: usize) -> Result<Self, String> {
        Ok(Self {
            logs: outputs.create()?,
            first,
            count: 0,
        })
    }

    fn push(&mut self, result: &Value) -> Result<(), String> {
        let mut text = result.pretty();
        text.push('\n');
        self.logs.stdout.write(text.as_bytes()).map_err(saving)?;
        self.count += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        self.logs.stdout.finish().map_err(saving)?;
        self.logs.stderr.finish().map_err(saving)
    }

    fn saved(&self) -> Saved {
        Saved {
            reference: self.logs.stdout_ref.clone(),
            first: self.first,
            count: self.count,
        }
    }
}

impl Saved {
    fn describe(self, fields: &mut std::collections::BTreeMap<String, Value>) {
        let first = self.first;
        let count = self.count;
        fields.insert("saved_files".into(), Value::number(count));
        fields.insert("result_file".into(), Value::string(self.reference));
        fields.insert(
            "notice".into(),
            Value::string(format!(
                "Every requested path has a result or error. files holds the first {first} results; the remaining {count}, from paths[{first}] on in call order, exceeded the inline size and are saved in result_file as one JSON object per path. Read result_file with offset and limit."
            )),
        );
    }
}

fn saving(error: std::io::Error) -> String {
    format!("Could not save read results: {error}")
}

fn failure(message: &str) -> Value {
    Value::object([("error", Value::string(message))])
}

/// Media are shown to the model only after a single read; other attachments
/// return their text page or path in this call.
pub(super) fn shown_alone(result: Value) -> Value {
    if result.get("view").is_some() {
        return failure("Media file: read it alone with path to view it");
    }
    result
}

/// The result named by the path the model asked for.
fn entry(path: &str, result: Value) -> Value {
    let Value::Object(mut fields) = result else {
        return result;
    };
    fields.insert("path".into(), Value::string(path));
    Value::Object(fields)
}

#[cfg(test)]
mod tests;
