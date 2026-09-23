use jecode::{
    json::{self, Value},
    state::Store,
};
use std::{
    io,
    path::Path,
    time::{Duration, SystemTime},
};

pub const OLDER: &str = "s-0000000000000001-00000000-00000000";
pub const NEWER: &str = "s-0000000000000002-00000000-00000000";
pub const OTHER: &str = "other-directory";

pub fn populate(home: &Path) -> io::Result<()> {
    std::fs::create_dir(home.join(OTHER))?;
    let store = Store::in_home(home)?.directory("sessions")?;
    for (id, title, age) in [
        (OLDER, "Older conversation", 3600),
        (
            NEWER,
            "Read settings.rs and run tests\nwithout repeating completed work",
            120,
        ),
    ] {
        let mut snapshot = json::parse(
            r#"{
            "version":1,"id":"","model":"gpt-5.6-luna","workspace":null,
            "file_access":"local","created":1,"updated":1,
            "history":[{
                "prompt":"","outcome":"Complete","end":"complete",
                "metrics":{"requests":0,"tool_calls":0,"elapsed_ms":0,
                    "approval_wait_ms":0,"first_text_ms":null,"input_tokens":null,
                    "output_tokens":null,"cached_tokens":null,"reasoning_tokens":null},
                "guidance":[],"steps":[]
            }],
            "projection":{"through":0,"summary":"","limit_bytes":524288,"failed":false}
        }"#,
            json::Limits::default(),
        )
        .unwrap();
        let Value::Object(fields) = &mut snapshot else {
            unreachable!()
        };
        for (key, value) in fields {
            match key.as_str() {
                "id" => *value = Value::String(id.into()),
                "workspace" => {
                    *value = Value::String(
                        (if id == OLDER {
                            home.join(OTHER)
                        } else {
                            home.to_owned()
                        })
                        .to_string_lossy()
                        .into_owned(),
                    )
                }
                "history" => {
                    let Value::Array(turns) = value else {
                        unreachable!()
                    };
                    let Value::Object(turn) = &mut turns[0] else {
                        unreachable!()
                    };
                    turn.insert("prompt".into(), Value::String(title.into()));
                }
                _ => {}
            }
        }
        let name = format!("{id}.json");
        store.replace(&name, &json::encode(&snapshot, 65536).unwrap())?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(store.root().join(name))?
            .set_modified(SystemTime::now() - Duration::from_secs(age))?;
    }
    Ok(())
}
