// Conversation messages painted by the real transcript renderer for review in
// a native terminal: cargo test --locked --offline specimen::messages -- --ignored --nocapture
use crate::tui::{
    line::Line,
    state::{Item, Kind, State, rows::Rows},
    theme::STRONG,
    view,
};

const WIDE: usize = 80;
const NARROW: usize = 40;
const EXTRA_WIDE: usize = 120;

const ANSWER: &str = r#"## Rate limiting for `/login`

I added a **fixed-window limiter** in front of the route. It keeps *one counter
per IP* and resets it every minute; see the [axum docs](https://docs.rs/axum)
for the middleware order. ~~The old sleep-based delay~~ is gone.

- `LOGIN_LIMIT` caps attempts at 5 per window
- Blocked requests get `429 Too Many Requests`
  - with a `Retry-After` header
  - and a log line at `warn`

1. Configure the window in `config.toml`
2. Restart the server

- [x] Limiter and tests
- [ ] Changelog entry

> Rate limiting alone won't stop credential stuffing; pair it with lockouts.

```rust
pub const LOGIN_LIMIT: u32 = 5;

fn allowed(count: u32) -> bool {
    count < LOGIN_LIMIT // "per window"
}
```

| Check | Result | Time |
| :--- | :---: | ---: |
| Unit tests | 12 passed | 0.8s |
| Integration | 3 passed | 4.1s |

---

Run it with:

```bash
cargo test --locked limiter -- --nocapture
```"#;

const FOLLOW_UP: &str = "Thanks. Can you also make the window configurable per route, so the admin\nlogin can be stricter than the public one?\n\nKeep the default at one minute.";

const SHORT: &str = r#"Yes. Each route now takes its own window:

```
[limits.login]
window = "60s"
attempts = 5
```

### Notes

The admin route uses `window = "300s"`. A very long line of prose follows to show how ordinary paragraphs wrap at the terminal width without any extra margins or indentation rules."#;

fn conversation() -> Vec<Item> {
    let text = |kind, text: &str| Item::Text {
        kind,
        text: text.into(),
    };
    vec![
        text(Kind::User, "add rate limiting to /login"),
        text(Kind::Assistant, ANSWER),
        text(Kind::User, FOLLOW_UP),
        text(Kind::Assistant, SHORT),
        Item::Streaming {
            text: "Checking the admin route now. The limiter reads its window from".into(),
        },
    ]
}

fn heading(rows: &mut Vec<(Line, usize)>, title: &str) {
    rows.push((Line::default(), WIDE));
    rows.push((Line::new(title, STRONG.bold()), WIDE));
    rows.push((Line::default(), WIDE));
}

#[test]
#[ignore = "prints the messages specimen for review in a native terminal"]
fn messages() {
    let mut rows = Vec::new();
    for columns in [NARROW, WIDE, EXTRA_WIDE] {
        heading(&mut rows, &format!("Messages  ·  {columns} columns"));
        let state = State {
            items: conversation(),
            ..State::default()
        };
        for line in view::opening::rows(&state, columns) {
            rows.push((line, columns));
        }
        let mut context = Rows::default();
        for item in conversation() {
            for line in context.render(&item, columns) {
                rows.push((line, columns));
            }
        }
    }
    let painted: String = rows
        .iter()
        .map(|(line, columns)| format!("{}\n", line.paint(*columns)))
        .collect();
    print!("{painted}");
}
