//! Local HTTP retrieval through the native OS adapter.

mod crawl;
mod html;
mod transport;
mod url;

#[cfg(test)]
mod tests;

use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use std::fs;
use std::io::Read;
use transport::{Options, Response};
use url::Url;

const PREVIEW_BYTES: usize = 6_000;
const EXTRACT_BYTES: usize = 4 * 1024 * 1024;
const LINK_PREVIEW: usize = 100;

pub fn execute(
    area: &Area,
    arguments: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    match arguments
        .get("action")
        .and_then(Value::as_str)
        .ok_or("action is required")?
    {
        "fetch" | "download" => {
            let action = arguments
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("");
            let url = Url::parse(
                arguments
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or("url is required")?,
            )?;
            let options = Options::from(arguments)?;
            let response = transport::fetch(
                area,
                &url,
                &options,
                std::slice::from_ref(&url),
                "web/fetches",
                cancellation,
            )?;
            describe(
                area,
                &url,
                &response,
                action == "fetch" && options.method != "HEAD",
                cancellation,
            )
        }
        "crawl" => crawl::execute(area, arguments, cancellation),
        _ => Err("action must be fetch, download, or crawl".into()),
    }
}

struct Description {
    value: Value,
    links: Vec<String>,
}

fn describe(
    area: &Area,
    requested: &Url,
    response: &Response,
    include_preview: bool,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    Ok(describe_page(area, requested, response, include_preview, cancellation)?.value)
}

fn describe_page(
    area: &Area,
    requested: &Url,
    response: &Response,
    include_preview: bool,
    cancellation: &Cancellation,
) -> Result<Description, String> {
    let mime = response
        .mime
        .as_deref()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let is_html = mime == "text/html" || mime == "application/xhtml+xml";
    let is_text = is_html
        || mime.starts_with("text/")
        || mime == "application/json"
        || mime.ends_with("+json")
        || mime == "application/xml"
        || mime.ends_with("+xml");
    let mut links = Vec::new();
    let mut title = None;
    let mut text_alias = None;
    let mut preview = None;
    let mut preview_truncated = false;
    let mut extraction_truncated = false;
    let mut links_alias = None;
    if is_text && include_preview {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&response.path)
            .map_err(|error| format!("Could not open response: {error}"))?
            .take(EXTRACT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read response: {error}"))?;
        extraction_truncated = bytes.len() > EXTRACT_BYTES;
        bytes.truncate(EXTRACT_BYTES);
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let text = if is_html {
            let document = html::extract(&source, &response.final_url);
            title = document.title;
            links = document.links;
            let alias = format!("{}.links.json", response.alias.trim_end_matches(".body"));
            let (_, path) = area.file(&alias, true)?;
            fs::write(
                &path,
                Value::Array(links.iter().map(Value::string).collect()).encode(),
            )
            .map_err(|error| format!("Could not save page links: {error}"))?;
            links_alias = Some(alias);
            document.text
        } else {
            source
        };
        let alias = format!("{}.txt", response.alias.trim_end_matches(".body"));
        let (_, path) = area.file(&alias, true)?;
        fs::write(&path, text.as_bytes())
            .map_err(|error| format!("Could not save page text: {error}"))?;
        preview_truncated = text.len() > PREVIEW_BYTES || extraction_truncated;
        preview = Some(first_bytes(&text, PREVIEW_BYTES));
        text_alias = Some(alias);
    }
    let mut fields = Value::object([
        ("requested_url", Value::string(&requested.value)),
        ("final_url", Value::string(&response.final_url.value)),
        ("status", Value::number(response.status)),
        (
            "mime",
            response.mime.as_ref().map_or(Value::Null, Value::string),
        ),
        ("fetched_at_unix", Value::number(response.fetched_at)),
        ("bytes", Value::number(response.bytes)),
        ("path", Value::string(format!("tmp:{}", response.alias))),
        (
            "local_path",
            Value::string(crate::scratch::environment_path(&response.path).to_string_lossy()),
        ),
        (
            "text_path",
            text_alias.map_or(Value::Null, |alias| Value::string(format!("tmp:{alias}"))),
        ),
        ("title", title.map_or(Value::Null, Value::string)),
        (
            "links",
            Value::Array(links.iter().take(LINK_PREVIEW).map(Value::string).collect()),
        ),
        ("links_count", Value::number(links.len())),
        (
            "links_path",
            links_alias.map_or(Value::Null, |alias| Value::string(format!("tmp:{alias}"))),
        ),
        ("extraction_truncated", Value::Bool(extraction_truncated)),
    ]);
    if let Some(preview) = preview
        && let Value::Object(map) = &mut fields
    {
        map.insert("preview".into(), Value::string(preview));
        map.insert("preview_truncated".into(), Value::Bool(preview_truncated));
    }
    Ok(Description {
        value: fields,
        links,
    })
}

fn first_bytes(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}
