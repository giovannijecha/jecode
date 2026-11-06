use super::PAGE;
use crate::browser::Registry;
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use crate::test_support::Directory;

pub(super) struct Session {
    pub(super) registry: Registry,
    area: Area,
    _directory: Directory,
}

impl Session {
    pub(super) fn new() -> Self {
        let directory = Directory::new();
        let area = Area::new(
            directory.path().join("123456-789"),
            directory.path().into(),
            "123456-789",
        )
        .unwrap();
        let session = Self {
            registry: Registry::default(),
            area,
            _directory: directory,
        };
        session.run("open", &[]);
        assert_eq!(session.value(PAGE), Value::Bool(true));
        session
    }

    pub(super) fn attempt(&self, action: &str, fields: &[(&str, Value)]) -> Result<Value, String> {
        let mut arguments = Value::object([("action", Value::string(action))]);
        if let Value::Object(entries) = &mut arguments {
            for (key, value) in fields {
                entries.insert((*key).into(), value.clone());
            }
        }
        self.registry
            .execute(&self.area, None, &arguments, &Cancellation::default())
    }

    pub(super) fn run(&self, action: &str, fields: &[(&str, Value)]) -> Value {
        self.attempt(action, fields)
            .unwrap_or_else(|error| panic!("{action}: {error}"))
    }

    pub(super) fn value(&self, expression: &str) -> Value {
        self.run("evaluate", &[("expression", Value::string(expression))])
            .get("value")
            .unwrap()
            .clone()
    }

    pub(super) fn press(&self, key: &str, modifiers: usize) {
        self.run(
            "press",
            &[
                ("key", Value::string(key)),
                ("modifiers", Value::number(modifiers)),
            ],
        );
    }

    pub(super) fn click(&self, selector: &str) {
        self.run("click", &[("selector", Value::string(selector))]);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.registry.stop_all();
    }
}
