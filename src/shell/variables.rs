use std::collections::HashMap;

/// Shell variables, all of which children receive as environment variables.
/// Names are case-insensitive on Windows, like its environment: `$PATH` reads
/// `Path`, and assigning `PATH` updates it under its original spelling.
#[derive(Clone, Default)]
pub(super) struct Variables(HashMap<String, (String, String)>);

impl Variables {
    pub fn from_environment() -> Self {
        let mut variables = Self::default();
        for (name, value) in std::env::vars_os() {
            if let (Ok(name), Ok(value)) = (name.into_string(), value.into_string()) {
                variables.set(&name, value);
            }
        }
        variables
    }

    fn key(name: &str) -> String {
        if cfg!(windows) {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(&Self::key(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(&Self::key(name))
    }

    pub fn set(&mut self, name: &str, value: String) {
        self.0
            .entry(Self::key(name))
            .and_modify(|entry| entry.1.clone_from(&value))
            .or_insert_with(|| (name.to_owned(), value));
    }

    pub fn remove(&mut self, name: &str) {
        self.0.remove(&Self::key(name));
    }

    /// Variables under their original names, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .values()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}
