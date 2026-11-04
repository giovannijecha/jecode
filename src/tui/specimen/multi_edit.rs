// Multi-file edit cards (B19) painted by the real card renderer for review in
// a native terminal:
// cargo test --locked --offline specimen::multi_edit -- --ignored --nocapture
use std::time::Duration;

use crate::events::tool_summary;
use crate::file_change::Change;
use crate::json::Value;
use crate::tui::{
    cards::{self, Presentation, Tool},
    line::Line,
    theme::{MUTED, STRONG},
};

const NOT_FOUND: &str = "edit 1: old text was not found; use exact current text from the supplied file context or read the file";
const TWICE: &str = "edit 1: old text occurs more than once; include more exact surrounding text from the supplied file context or read the file";

/// One file of the call: its replacement, or the error it failed with.
struct File {
    path: &'static str,
    before: String,
    old: String,
    new: String,
    error: Option<&'static str>,
}

fn file(path: &'static str, before: &[&str], old: &str, new: &str) -> File {
    File {
        path,
        before: before.iter().map(|line| format!("{line}\n")).collect(),
        old: old.into(),
        new: new.into(),
        error: None,
    }
}

fn failed(path: &'static str, error: &'static str) -> File {
    File {
        path,
        before: String::new(),
        old: "missing".into(),
        new: "unused".into(),
        error: Some(error),
    }
}

/// Refresh tokens 30 s before they expire: three files.
fn three() -> Vec<File> {
    vec![
        file(
            "src/auth/session.ts",
            &[
                "import type { Session } from \"./types\";",
                "import * as token from \"./token\";",
                "",
                "/** Renews the access token when it is about to expire. */",
                "export function refreshIfDue(session: Session): void {",
                "  const now = Date.now();",
                "  if (session.expiresAt < now) token.refresh(session);",
                "}",
            ],
            "  if (session.expiresAt < now) token.refresh(session);",
            "  if (session.expiresAt - token.SKEW_MS <= now) token.refresh(session);",
        ),
        file(
            "src/auth/token.ts",
            &[
                "// Access tokens from the identity service.",
                "export const TTL_SECONDS = 3600;",
                "",
                "export async function refresh(session: Session): Promise<void> {",
                "  const next = await identity.renew(session.refreshToken);",
                "  session.accessToken = next.accessToken;",
                "  session.expiresAt = next.expiresAt;",
                "}",
            ],
            "export const TTL_SECONDS = 3600;\n",
            "export const TTL_SECONDS = 3600;\n// Refresh this long before expiry, so requests in flight keep a valid token.\nexport const SKEW_MS = 30_000;\n",
        ),
        file(
            "tests/auth/expiry.test.ts",
            &[
                "import { refreshIfDue } from \"../../src/auth/session\";",
                "import * as token from \"../../src/auth/token\";",
                "",
                "describe(\"refreshIfDue\", () => {",
                "  it(\"refreshes an expired session\", () => {",
                "    const spy = vi.spyOn(token, \"refresh\").mockResolvedValue();",
                "    refreshIfDue({ expiresAt: Date.now() - 1 } as Session);",
                "    expect(spy).toHaveBeenCalledOnce();",
                "  });",
                "});",
            ],
            "  });\n});\n",
            "  });\n\n  it(\"refreshes 30 s before expiry\", () => {\n    const spy = vi.spyOn(token, \"refresh\").mockResolvedValue();\n    refreshIfDue({ expiresAt: Date.now() + 29_000 } as Session);\n    expect(spy).toHaveBeenCalledOnce();\n  });\n});\n",
        ),
    ]
}

/// The same call with `token.ts` failing.
fn partial() -> Vec<File> {
    let mut files = three();
    files[1] = failed("src/auth/token.ts", NOT_FOUND);
    files
}

/// Rename `refreshIfDue` across nine files; the docs fail.
fn many() -> Vec<File> {
    let rename = |path: &'static str, rows: &[&str]| {
        let line = rows
            .iter()
            .find(|row| row.contains("refreshIfDue"))
            .unwrap();
        file(
            path,
            rows,
            line,
            &line.replace("refreshIfDue", "refreshBeforeExpiry"),
        )
    };
    vec![
        rename(
            "src/auth/session.ts",
            &[
                "import * as token from \"./token\";",
                "",
                "export function refreshIfDue(session: Session): void {",
                "  const now = Date.now();",
            ],
        ),
        rename(
            "src/auth/index.ts",
            &[
                "export { refresh } from \"./token\";",
                "export { refreshIfDue } from \"./session\";",
            ],
        ),
        rename(
            "src/api/client.ts",
            &[
                "  const session = await sessions.current();",
                "  auth.refreshIfDue(session);",
                "  const headers = new Headers(init.headers);",
            ],
        ),
        rename(
            "src/api/retry.ts",
            &[
                "  const session = await sessions.current();",
                "  auth.refreshIfDue(session);",
                "  return again();",
            ],
        ),
        rename(
            "src/components/settings/SessionExpiryNotice.tsx",
            &[
                "  const renew = () => auth.refreshIfDue(session);",
                "  return <Notice tone=\"quiet\" action={renew}>Signed in</Notice>;",
            ],
        ),
        rename(
            "src/workers/refresh.worker.ts",
            &[
                "  const session = event.data as Session;",
                "  auth.refreshIfDue(session);",
                "  self.postMessage(session);",
            ],
        ),
        rename(
            "tests/auth/session.test.ts",
            &["", "const subject = auth.refreshIfDue;", ""],
        ),
        rename(
            "tests/api/client.test.ts",
            &["  vi.spyOn(auth, \"refreshIfDue\").mockReturnValue(undefined);"],
        ),
        failed("docs/AUTH.md", TWICE),
    ]
}

/// A finished, running or expanded call over `files`.
struct Call {
    arguments: Value,
    result: Option<Value>,
    change: Option<Change>,
    presentation: Presentation,
}

fn call(files: &[File], running: bool, expanded: bool) -> Call {
    let arguments = Value::object([(
        "files",
        Value::Array(
            files
                .iter()
                .map(|file| {
                    Value::object([
                        ("path", Value::string(file.path)),
                        (
                            "edits",
                            Value::Array(vec![Value::object([
                                ("old_text", Value::string(&file.old)),
                                ("new_text", Value::string(&file.new)),
                            ])]),
                        ),
                    ])
                })
                .collect(),
        ),
    )]);
    let mut changes = Vec::new();
    let entries = files
        .iter()
        .map(|file| {
            if let Some(error) = file.error {
                return Value::object([
                    ("path", Value::string(file.path)),
                    ("error", Value::string(error)),
                ]);
            }
            let after = file.before.replacen(&file.old, &file.new, 1);
            if let Some(change) = Change::between(Some(&file.before), &after) {
                changes.push((file.path.to_owned(), change));
            }
            Value::object([
                ("path", Value::string(file.path)),
                ("bytes_written", Value::number(after.len())),
            ])
        })
        .collect();
    Call {
        arguments,
        result: (!running).then(|| Value::object([("files", Value::Array(entries))])),
        change: (!running).then(|| Change::several(changes)).flatten(),
        presentation: Presentation {
            elapsed: Some(Duration::from_millis(if running { 400 } else { 9 })),
            expanded,
            ..Presentation::default()
        },
    }
}

fn rows(call: &Call, columns: usize, keycap: bool) -> Vec<Line> {
    let summary = call
        .result
        .as_ref()
        .map(|result| tool_summary("edit", result))
        .unwrap_or_default();
    let tool = Tool {
        name: "edit",
        arguments: &call.arguments,
        summary: &summary,
        result: call.result.as_ref(),
        change: call.change.as_ref(),
        presentation: &call.presentation,
    };
    cards::card(tool, columns, keycap).rows
}

#[test]
#[ignore = "prints the multi-file edit specimen for review in a native terminal"]
fn multi_edit() {
    let nothing: Vec<_> = partial()
        .into_iter()
        .map(|file| failed(file.path, file.error.unwrap_or(NOT_FOUND)))
        .collect();
    let states = [
        ("Collapsed, three files", call(&three(), false, false), true),
        ("Running", call(&three(), true, false), false),
        ("Expanded, Ctrl+O", call(&three(), false, true), true),
        ("Partial failure", call(&partial(), false, false), true),
        (
            "Partial failure expanded",
            call(&partial(), false, true),
            true,
        ),
        ("Nothing changed", call(&nothing, false, false), true),
        ("Nine files", call(&many(), false, false), true),
        ("Nine files expanded", call(&many(), false, true), true),
    ];
    let mut painted = String::new();
    for columns in [44, 80, 100] {
        for (title, call, keycap) in &states {
            painted.push_str(&format!(
                "{}\n{}\n",
                Line::new(&format!("{title}  ·  {columns} columns"), STRONG.bold()).paint(columns),
                Line::new(&"─".repeat(columns), MUTED).paint(columns)
            ));
            for line in rows(call, columns, *keycap) {
                painted.push_str(&line.paint(columns));
                painted.push('\n');
            }
            painted.push('\n');
        }
    }
    print!("{painted}");
}
