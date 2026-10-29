//! External access: HTTP fetches and crawls, and local Chromium control.

use super::{boolean, integer, routine, string, strings};
use crate::json::Value;

pub(super) fn web() -> Value {
    routine(
        "web",
        "Use native OS HTTP/TLS for HTTP(S) fetch, raw download or link crawl. fetch returns status, final_url, MIME, raw/text paths and preview; extraction stops at 4 MiB (see extraction_truncated). HTML returns 100 links at most; links_count/links_path cover all. download saves raw bytes. fetch/download need url and accept method, headers, body; headers stay at the requested origin on redirects. crawl uses GET without body: start with urls or resume with crawl_id and unchanged saved seed headers, timeout_seconds and max_bytes. Defaults: same_origin true, max_depth 1, max_pages 20 per call. Returns page summaries plus manifest_path for full JSONL records; page with read. max_bytes limits each response only when set. Use browser for rendered pages or search sites.",
        Value::object([
            (
                "action",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(["fetch", "download", "crawl"].map(Value::string).into()),
                    ),
                ]),
            ),
            ("url", string()),
            ("urls", strings()),
            ("crawl_id", string()),
            ("method", string()),
            (
                "headers",
                Value::object([
                    ("type", Value::string("object")),
                    ("additionalProperties", string()),
                ]),
            ),
            ("body", string()),
            ("timeout_seconds", integer(1, None)),
            ("max_bytes", integer(1, None)),
            ("max_pages", integer(1, None)),
            ("max_depth", integer(0, None)),
            ("same_origin", boolean()),
        ]),
        &["action"],
    )
}

pub(super) fn browser() -> Value {
    routine(
        "browser",
        "Control installed Chromium locally via CDP. open defaults to isolated headless mode; visible:true opens a window, and profile names persistent profiles (default temporary). width+height set the page viewport on open (before url) or screenshot. Omitted browser_id selects newest; omitted tab_id selects an available tab, so specify it when needed. snapshot gives elements with refs such as @1, role, name and state; selector accepts refs or CSS. wait uses selector/text. screenshot saves tmp:browser/, delivers an attachment next request, and the ref survives temporary cleanup. logs pages events (default console and errors); pass next_cursor as cursor; missed counts evicted events. cdp sends a raw DevTools method. Use visible mode for login or two-factor prompts. Chromium must be installed; JECODE_BROWSER can name its executable.",
        Value::object([
            (
                "action",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(
                            [
                                "open",
                                "tabs",
                                "navigate",
                                "snapshot",
                                "click",
                                "fill",
                                "press",
                                "scroll",
                                "wait",
                                "screenshot",
                                "evaluate",
                                "cdp",
                                "logs",
                                "close",
                                "stop",
                            ]
                            .map(Value::string)
                            .into(),
                        ),
                    ),
                ]),
            ),
            ("url", string()),
            ("visible", boolean()),
            ("profile", string()),
            ("browser_id", string()),
            ("tab_id", string()),
            ("selector", string()),
            ("text", string()),
            ("key", string()),
            ("expression", string()),
            ("method", string()),
            ("params", Value::object([("type", Value::string("object"))])),
            ("timeout_ms", integer(1, None)),
            ("x", integer(0, None)),
            ("y", integer(0, None)),
            (
                "delta_y",
                Value::object([("type", Value::string("integer"))]),
            ),
            ("width", integer(1, Some(10_000))),
            ("height", integer(1, Some(10_000))),
            (
                "filter",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(
                            ["default", "console", "errors", "network", "all"]
                                .map(Value::string)
                                .into(),
                        ),
                    ),
                ]),
            ),
            ("limit", integer(1, Some(200))),
            ("cursor", string()),
        ]),
        &["action"],
    )
}
