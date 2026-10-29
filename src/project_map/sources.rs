//! Bounded full-content fingerprints; freshness only covers referenced files.
use crate::{cancel::Cancellation, disk::hash, json::Value, redact::Redactor};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

const BUDGET: u64 = 64 * 1024 * 1024;

pub(super) struct Sources<'a> {
    root: &'a Path,
    cancellation: &'a Cancellation,
    remaining: u64,
    checked: BTreeMap<String, Result<String, String>>,
}

impl<'a> Sources<'a> {
    pub fn new(root: &'a Path, cancellation: &'a Cancellation) -> Self {
        Self {
            root,
            cancellation,
            remaining: BUDGET,
            checked: BTreeMap::new(),
        }
    }

    pub fn capture(&mut self, value: &Value, redactor: &Redactor) -> Result<Value, String> {
        let path = crate::tools::required_string(value, "path")?;
        let line = match value.get("line") {
            None => None,
            Some(value) => Some(
                value
                    .as_usize()
                    .filter(|&line| line > 0)
                    .ok_or("source line must be a positive integer")?,
            ),
        };
        let (path, fingerprint, excerpt) = self.read(path, line)?;
        let mut fields = BTreeMap::from([
            ("path".into(), Value::string(path)),
            ("fingerprint".into(), Value::string(fingerprint)),
            ("status".into(), Value::string("current")),
        ]);
        if let Some(line) = line {
            fields.insert("line".into(), Value::number(line));
            fields.insert("text".into(), Value::string(redactor.text(&excerpt)));
        }
        Ok(Value::Object(fields))
    }

    pub fn refresh(&mut self, note: &Value) -> Result<Value, String> {
        let mut note = note.clone();
        let Value::Object(fields) = &mut note else {
            return Err("Invalid project map note".into());
        };
        let Some(Value::Array(sources)) = fields.get_mut("sources") else {
            return Err("Invalid project map sources".into());
        };
        let mut freshness = "current";
        for source in sources {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let path = source
                .get("path")
                .and_then(Value::as_str)
                .ok_or("Invalid project map source path")?
                .to_owned();
            let fingerprint = source.get("fingerprint").and_then(Value::as_str);
            if !self.checked.contains_key(&path) {
                let result = self.read(&path, None).map(|(_, digest, _)| digest);
                self.checked.insert(path.clone(), result);
            }
            let (status, error) = match &self.checked[&path] {
                Ok(current) if Some(current.as_str()) == fingerprint => ("current", None),
                Ok(_) => ("changed", None),
                Err(error) if error == "Source file is missing" => ("missing", Some(error)),
                Err(error) => ("unverified", Some(error)),
            };
            if rank(status) > rank(freshness) {
                freshness = status;
            }
            if let Value::Object(fields) = source {
                fields.insert("status".into(), Value::string(status));
                if let Some(error) = error {
                    fields.insert("error".into(), Value::string(error));
                }
            }
        }
        fields.insert("freshness".into(), Value::string(freshness));
        Ok(note)
    }

    fn read(
        &mut self,
        requested: &str,
        line: Option<usize>,
    ) -> Result<(String, String, String), String> {
        if self.cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let absolute = std::fs::canonicalize(self.root.join(requested)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Source file is missing".into()
            } else {
                format!("Could not inspect source: {error}")
            }
        })?;
        let relative = absolute
            .strip_prefix(self.root)
            .map_err(|_| "Source path is outside the workspace")?;
        let path = relative
            .to_str()
            .ok_or("Source path is not UTF-8")?
            .replace('\\', "/");
        if !std::fs::metadata(&absolute)
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("Source must be a regular file".into());
        }
        let mut file =
            File::open(&absolute).map_err(|error| format!("Could not read source: {error}"))?;
        let before = file.metadata().map_err(|error| error.to_string())?;
        if !before.is_file() {
            return Err("Source must be a regular file".into());
        }
        if before.len() > self.remaining {
            return Err("Source fingerprint budget exhausted (64 MiB per call)".into());
        }
        self.remaining -= before.len();
        let mut hasher = hash::Hasher::new(before.len());
        let mut remaining = before.len();
        let mut buffer = [0u8; 64 * 1024];
        let mut line_number = 1usize;
        let mut excerpt = Vec::new();
        let mut selected = false;
        while remaining > 0 {
            if self.cancellation.requested() {
                return Err("Operation cancelled".into());
            }
            let count = file
                .read(&mut buffer[..remaining.min(64 * 1024) as usize])
                .map_err(|error| error.to_string())?;
            if count == 0 {
                return Err("Source changed while being read".into());
            }
            remaining -= count as u64;
            hasher.update(&buffer[..count]);
            if let Some(line) = line {
                for &byte in &buffer[..count] {
                    if line_number == line {
                        selected = true;
                        if excerpt.len() < 640 && byte != b'\n' {
                            excerpt.push(byte);
                        }
                    }
                    if byte == b'\n' {
                        line_number += 1;
                    }
                }
            }
        }
        let after = file.metadata().map_err(|error| error.to_string())?;
        let current = std::fs::metadata(&absolute).map_err(|error| error.to_string())?;
        if before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || before.len() != current.len()
            || before.modified().ok() != current.modified().ok()
        {
            return Err("Source changed while being read".into());
        }
        if line.is_some() && !selected {
            return Err("Source line is outside the file".into());
        }
        let digest = hasher.finish().0;
        Ok((
            path,
            format!("{:016x}{:016x}", digest[0], digest[1]),
            String::from_utf8_lossy(&excerpt)
                .trim_end_matches('\r')
                .chars()
                .take(160)
                .collect(),
        ))
    }
}

fn rank(status: &str) -> u8 {
    match status {
        "missing" => 3,
        "changed" => 2,
        "unverified" => 1,
        _ => 0,
    }
}
