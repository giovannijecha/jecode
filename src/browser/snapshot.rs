//! Page text plus interactive elements with browser-computed semantics.
use super::{
    actions::{DEFAULT_TIMEOUT, evaluate, output_path},
    transport::Cdp,
};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use std::fs;

const TEXT_LIMIT: usize = 12000;
const SHOWN_ELEMENTS: usize = 100;
/// Elements that receive Chromium accessibility details, one call each.
const ACCESSIBLE_ELEMENTS: usize = 500;
const GROUP: &str = "jecode-snapshot";

/// Collects elements in document order and keeps them for `@N` refs. DOM
/// fields describe identity and current state; empty fields are omitted.
const SCRIPT: &str = r#"(()=>{const nodes=[...document.querySelectorAll('a,button,input,textarea,select,[role="button"],[contenteditable="true"]')];Object.defineProperty(window,Symbol.for('jecode.browser.refs'),{value:nodes,configurable:true});const plain=new Set(['checkbox','radio','button','submit','reset','image','file','password','hidden']);const clip=(v)=>{let out='',n=0;for(const c of v){if(n>=200)break;out+=c;n++}return out};const els=nodes.map((e,i)=>{const r=e.getBoundingClientRect(),tag=e.tagName.toLowerCase(),type=(e.getAttribute('type')||'').toLowerCase(),o={ref:'@'+(i+1),tag,visible:r.width>0&&r.height>0};const put=(k,v)=>{if(typeof v!=='string')return;v=v.trim();if(v)o[k]=clip(v)};put('type',type);put('role',e.getAttribute('role'));put('id',e.id);put('html_name',e.getAttribute('name'));put('placeholder',e.getAttribute('placeholder'));if(tag!=='select'&&tag!=='textarea')put('text',e.innerText);if(tag==='textarea'||tag==='select'||(tag==='input'&&!plain.has(type))){const v=String(e.value);if(v.length)o.value=clip(v)}if(tag==='input'&&(type==='checkbox'||type==='radio'))o.checked=e.checked;if(e.disabled===true)o.disabled=true;return o});return {title:document.title,url:location.href,text:document.body?.innerText||'',elements:els}})()"#;

pub(super) fn capture(
    connection: &mut Cdp,
    area: &Area,
    id: &str,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let mut value = evaluate(connection, SCRIPT, cancellation, DEFAULT_TIMEOUT)?;
    if let Some(Value::Array(elements)) = match &mut value {
        Value::Object(map) => map.get_mut("elements"),
        _ => None,
    } {
        let outcome = semantics(connection, elements, cancellation);
        if outcome.as_ref().is_err_and(|e| e == "Operation cancelled") {
            return outcome.map(|_| Value::Null);
        }
        if let (Err(error), Value::Object(map)) = (outcome, &mut value) {
            map.insert("accessibility_error".into(), Value::string(error));
        }
    }
    let (path, relative) = output_path(area, "json")?;
    fs::write(path, value.encode()).map_err(|e| e.to_string())?;
    let mut short = value;
    if let Value::Object(ref mut map) = short {
        if let Some(Value::String(text)) = map.get_mut("text") {
            truncate_text(text);
        }
        if let Some(Value::Array(elements)) = map.get_mut("elements")
            && elements.len() > SHOWN_ELEMENTS
        {
            let total = elements.len();
            elements.truncate(SHOWN_ELEMENTS);
            map.insert("elements_total".into(), Value::number(total));
        }
        map.insert("snapshot_path".into(), Value::string(relative));
        map.insert("tab_id".into(), Value::string(id));
    }
    Ok(short)
}

fn truncate_text(text: &mut String) {
    if let Some((end, _)) = text.char_indices().nth(TEXT_LIMIT) {
        text.truncate(end);
        text.push_str("\n[truncated; full snapshot saved]");
    }
}

/// Adds Chromium's computed role, accessible name and states to the elements
/// behind the refs. Names cover labels, wrapping labels and ARIA naming.
fn semantics(
    connection: &mut Cdp,
    elements: &mut [Value],
    cancellation: &Cancellation,
) -> Result<(), String> {
    let handle = connection.call(
        "Runtime.evaluate",
        Value::object([
            (
                "expression",
                Value::string(format!(
                    "window[Symbol.for('jecode.browser.refs')]?.slice(0,{ACCESSIBLE_ELEMENTS})"
                )),
            ),
            ("objectGroup", Value::string(GROUP)),
        ]),
        cancellation,
        DEFAULT_TIMEOUT,
    )?;
    let outcome = handle
        .get("result")
        .and_then(|result| result.get("objectId"))
        .and_then(Value::as_str)
        .ok_or_else(|| "Snapshot elements are unavailable".to_string())
        .and_then(|array| annotate(connection, array, elements, cancellation));
    if connection.usable() {
        let _ = connection.call(
            "Runtime.releaseObjectGroup",
            Value::object([("objectGroup", Value::string(GROUP))]),
            cancellation,
            DEFAULT_TIMEOUT,
        );
    }
    outcome
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;

fn annotate(
    connection: &mut Cdp,
    array: &str,
    elements: &mut [Value],
    cancellation: &Cancellation,
) -> Result<(), String> {
    let properties = connection.call(
        "Runtime.getProperties",
        Value::object([
            ("objectId", Value::string(array)),
            ("ownProperties", Value::Bool(true)),
        ]),
        cancellation,
        DEFAULT_TIMEOUT,
    )?;
    for property in properties
        .get("result")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        let Some(index) = property
            .get("name")
            .and_then(Value::as_str)
            .and_then(|name| name.parse::<usize>().ok())
            .filter(|index| *index < elements.len().min(ACCESSIBLE_ELEMENTS))
        else {
            continue;
        };
        let Some(object) = property
            .get("value")
            .and_then(|value| value.get("objectId"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let tree = connection.call(
            "Accessibility.getPartialAXTree",
            Value::object([
                ("objectId", Value::string(object)),
                ("fetchRelatives", Value::Bool(false)),
            ]),
            cancellation,
            DEFAULT_TIMEOUT,
        )?;
        if let Some(node) = tree
            .get("nodes")
            .and_then(Value::as_array)
            .and_then(|nodes| nodes.first())
        {
            apply(&mut elements[index], node);
        }
    }
    Ok(())
}

fn apply(element: &mut Value, node: &Value) {
    let Value::Object(fields) = element else {
        return;
    };
    if node.get("ignored") == Some(&Value::Bool(true)) {
        return;
    }
    let value = |key: &str| {
        node.get(key)
            .and_then(|field| field.get("value"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    if let Some(role) = value("role") {
        fields.insert("role".into(), Value::string(role));
    }
    if let Some(name) = value("name") {
        fields.insert(
            "name".into(),
            Value::string(name.chars().take(200).collect::<String>()),
        );
        if fields.get("text") == fields.get("name") {
            fields.remove("text");
        }
    }
    for property in node
        .get("properties")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        let state = property.get("value").and_then(|value| value.get("value"));
        match (property.get("name").and_then(Value::as_str), state) {
            (Some("disabled"), Some(Value::Bool(true))) => {
                fields.insert("disabled".into(), Value::Bool(true));
            }
            (Some("checked"), Some(Value::String(checked))) => {
                let checked = match checked.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    other => Value::string(other),
                };
                fields.insert("checked".into(), checked);
            }
            _ => {}
        }
    }
}
