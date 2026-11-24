//! Recovering tool calls that a model wrote as text.
//!
//! Small local models (Qwen 2.5 Coder on Ollama, for example) often answer a
//! tool-calling request with the call printed as JSON instead of a structured
//! call. Left alone, nothing runs and the user sees raw JSON. A reply is turned
//! into a call only when the *whole* reply is one JSON call and it names a tool
//! that was actually offered, so prose that merely contains JSON is never run.

use serde_json::{Map, Value};

/// One recovered call: the offered tool's name and its JSON object arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct TextToolCall {
    pub name: String,
    pub args: Value,
}

/// The call `text` spells out, if it is nothing but a call to one of `offered`.
///
/// Accepts a bare JSON object, the same inside a Markdown code fence or
/// `<tool_call>` tags, or a one-element array of it. Arguments may sit under
/// `arguments`, `parameters` or `args`, as an object or a JSON-encoded string;
/// the OpenAI-style `{"function": {...}}` wrapper is unwrapped.
pub fn parse_text_tool_call<'a>(
    text: &str,
    offered: impl IntoIterator<Item = &'a str>,
) -> Option<TextToolCall> {
    let offered: Vec<&str> = offered.into_iter().collect();
    if let Some(call) = xml_call(text.trim()) {
        return offered.contains(&call.name.as_str()).then_some(call);
    }
    let body = strip_wrappers(text.trim())?;
    let value: Value = serde_json::from_str(body).ok()?;
    let object = match value {
        Value::Object(object) => object,
        Value::Array(mut items) if items.len() == 1 => match items.pop() {
            Some(Value::Object(object)) => object,
            _ => return None,
        },
        _ => return None,
    };
    let object = match object.get("function") {
        Some(Value::Object(inner)) if !object.contains_key("name") => inner.clone(),
        _ => object,
    };
    let name = object.get("name")?.as_str()?.trim();
    if !offered.contains(&name) {
        return None;
    }
    let args = arguments(&object)?;
    Some(TextToolCall {
        name: name.to_owned(),
        args,
    })
}

/// A lead-in longer than this is an answer that mentions JSON, not a call.
const MAX_LEAD_IN_CHARS: usize = 600;
/// Tool arguments are small; a longer block is content (code, data), not a call.
const MAX_CALL_CHARS: usize = 4_000;

/// True when `tail` could still become a call once more of it streams in, so a
/// streaming adapter should keep holding it back instead of showing it.
pub fn may_be_text_tool_call(tail: &str) -> bool {
    let trimmed = tail.trim_start();
    if trimmed.len() > MAX_CALL_CHARS {
        return false;
    }
    let body = if let Some(fenced) = trimmed.strip_prefix("```") {
        // Only an untagged or `json` fence can hold a call; ```rust is code.
        match fenced.split_once('\n') {
            Some((info, rest)) if matches!(info.trim(), "" | "json") => rest,
            Some(_) => return false,
            None => return "json".starts_with(fenced.trim()),
        }
    } else if let Some(rest) = trimmed.strip_prefix("<tool_call>") {
        rest
    } else if "<tool_call>".starts_with(trimmed) || "```".starts_with(trimmed) {
        return true;
    } else {
        trimmed
    };
    json_call_prefix(body) || xml_call_prefix(body)
}

/// True while `body` still reads like the opening of `<function=`.
fn xml_call_prefix(body: &str) -> bool {
    let body = body.trim_start();
    body.starts_with(XML_FUNCTION) || XML_FUNCTION.starts_with(body)
}

const XML_FUNCTION: &str = "<function=";
const XML_PARAMETER: &str = "<parameter=";

/// Qwen 3 Coder's own call format, also when the model drops closing tags:
/// `<tool_call><function=NAME><parameter=KEY>VALUE</parameter></function></tool_call>`.
/// A value that parses as JSON (a number, list or object) keeps that type.
fn xml_call(text: &str) -> Option<TextToolCall> {
    let text = text
        .strip_prefix("<tool_call>")
        .unwrap_or(text)
        .trim_start();
    let text = text.strip_suffix("</tool_call>").unwrap_or(text).trim_end();
    let text = text.strip_suffix("</function>").unwrap_or(text);
    let rest = text.strip_prefix(XML_FUNCTION)?;
    let (name, mut rest) = rest.split_once('>')?;
    let name = name.trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let mut args = Map::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let after = rest.strip_prefix(XML_PARAMETER)?;
        let (key, after) = after.split_once('>')?;
        let end = [after.find("</parameter>"), after.find(XML_PARAMETER)]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(after.len());
        let raw = after[..end].trim();
        let value = serde_json::from_str(raw)
            .ok()
            .filter(|v: &Value| !v.is_string())
            .unwrap_or_else(|| Value::String(raw.to_owned()));
        args.insert(key.trim().to_owned(), value);
        rest = after[end..]
            .strip_prefix("</parameter>")
            .unwrap_or(&after[end..]);
    }
    Some(TextToolCall {
        name: name.to_owned(),
        args: Value::Object(args),
    })
}

/// Byte offset of the earliest line at or after `from` where a text call
/// could begin, if any. Inline braces in prose (`use {x}`) never qualify.
pub fn text_tool_call_start(text: &str, from: usize) -> Option<usize> {
    line_starts(text)
        .filter(|&start| start >= from)
        .find(|&start| {
            let line = text[start..].trim_start_matches([' ', '\t']);
            !line.is_empty() && !line.starts_with(['\n', '\r']) && may_be_text_tool_call(line)
        })
}

/// True while `body` still reads like the opening of `{"name"…` or `{"function"…`.
fn json_call_prefix(body: &str) -> bool {
    let compact: String = body
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(13)
        .collect();
    ["{\"name\"", "{\"function\"", "[{\"name\"", "[{\"function\""]
        .iter()
        .any(|opening| opening.starts_with(compact.as_str()) || compact.starts_with(opening))
}

/// A reply that ends with one call to an offered tool, after at most a short
/// lead-in sentence: returns the lead-in's byte length and the call.
pub fn split_text_tool_call(text: &str, offered: &[&str]) -> Option<(usize, TextToolCall)> {
    line_starts(text)
        .take_while(|&start| text[..start].chars().count() <= MAX_LEAD_IN_CHARS)
        // Blank lines are part of the lead-in, so the call starts where its text does.
        .filter(|&start| {
            !text[start..]
                .trim_start_matches([' ', '\t'])
                .starts_with(['\n', '\r'])
        })
        .find_map(|start| {
            parse_text_tool_call(&text[start..], offered.iter().copied()).map(|call| (start, call))
        })
}

fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.match_indices('\n').map(|(index, _)| index + 1))
}

fn strip_wrappers(text: &str) -> Option<&str> {
    let text = text
        .strip_prefix("<tool_call>")
        .and_then(|rest| rest.strip_suffix("</tool_call>"))
        .map_or(text, str::trim);
    if let Some(fenced) = text.strip_prefix("```") {
        let fenced = fenced.strip_suffix("```")?;
        // Drop an info string such as `json` on the opening fence line.
        let body = fenced.split_once('\n').map_or(fenced, |(first, rest)| {
            if first.trim().chars().all(|c| c.is_ascii_alphanumeric()) {
                rest
            } else {
                fenced
            }
        });
        return Some(body.trim());
    }
    Some(text)
}

fn arguments(object: &Map<String, Value>) -> Option<Value> {
    let raw = ["arguments", "parameters", "args"]
        .iter()
        .find_map(|key| object.get(*key));
    match raw {
        None | Some(Value::Null) => Some(Value::Object(Map::new())),
        Some(Value::Object(args)) => Some(Value::Object(args.clone())),
        Some(Value::String(encoded)) => match serde_json::from_str(encoded).ok()? {
            Value::Object(args) => Some(Value::Object(args)),
            _ => None,
        },
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const OFFERED: [&str; 2] = ["WebSearchAgent", "web_search"];

    #[test]
    fn bare_json_call_to_an_offered_tool_is_recovered() {
        let text =
            r#"{"name": "WebSearchAgent", "arguments": {"request": "Official Rust website"}}"#;
        assert_eq!(
            parse_text_tool_call(text, OFFERED),
            Some(TextToolCall {
                name: "WebSearchAgent".to_owned(),
                args: json!({ "request": "Official Rust website" }),
            })
        );
    }

    #[test]
    fn fenced_tagged_array_and_string_argument_forms_are_recovered() {
        for text in [
            "```json\n{\"name\": \"web_search\", \"parameters\": {\"queries\": [\"rust\"]}}\n```",
            "<tool_call>{\"name\": \"web_search\", \"args\": {\"queries\": [\"rust\"]}}</tool_call>",
            "[{\"name\": \"web_search\", \"arguments\": {\"queries\": [\"rust\"]}}]",
            "{\"name\": \"web_search\", \"arguments\": \"{\\\"queries\\\": [\\\"rust\\\"]}\"}",
            "{\"function\": {\"name\": \"web_search\", \"arguments\": {\"queries\": [\"rust\"]}}}",
        ] {
            let call = parse_text_tool_call(text, OFFERED).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(call.name, "web_search");
            assert_eq!(call.args, json!({ "queries": ["rust"] }), "{text}");
        }
    }

    /// Seen live with qwen3-coder:30b on Ollama: its own format, closing tags dropped.
    #[test]
    fn qwen_xml_calls_are_recovered_even_without_closing_tags() {
        let offered = ["search_saved_details", "list_file_findings"];
        for text in [
            "<function=search_saved_details> <parameter=query> What temperature did the thermometer read?  </tool_call>",
            "<tool_call>\n<function=search_saved_details>\n<parameter=query>\nWhat temperature did the thermometer read?\n</parameter>\n</function>\n</tool_call>",
        ] {
            let call = parse_text_tool_call(text, offered).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(call.name, "search_saved_details");
            assert_eq!(
                call.args,
                json!({ "query": "What temperature did the thermometer read?" })
            );
        }
        let typed = parse_text_tool_call(
            "<function=list_file_findings><parameter=kind>measurement</parameter><parameter=next>7</parameter></function>",
            offered,
        )
        .expect("typed call");
        assert_eq!(typed.args, json!({ "kind": "measurement", "next": 7 }));
        assert!(parse_text_tool_call("<function=remove_all></function>", offered).is_none());
        assert!(
            parse_text_tool_call("Use <function=search_saved_details> later", offered).is_none()
        );
        assert!(may_be_text_tool_call("<funct"));
        assert!(may_be_text_tool_call("<tool_call>\n<function=search"));
        let lead = "I need to search your saved details.\n\n<function=search_saved_details> <parameter=query> thermometer </tool_call>";
        let (start, call) = split_text_tool_call(lead, &offered).expect("lead-in call");
        assert!(lead[..start].starts_with("I need to search"));
        assert_eq!(call.args, json!({ "query": "thermometer" }));
    }

    #[test]
    fn prose_unknown_tools_and_bad_arguments_are_left_as_text() {
        for text in [
            r#"Here is the call: {"name": "web_search", "arguments": {}}"#,
            r#"{"name": "delete_everything", "arguments": {}}"#,
            r#"{"name": "web_search", "arguments": [1, 2]}"#,
            r#"[{"name": "web_search"}, {"name": "web_search"}]"#,
            "Rust 1.90 was released in September.",
            "{not json}",
        ] {
            assert_eq!(parse_text_tool_call(text, OFFERED), None, "{text}");
        }
        // No arguments at all is a call with empty arguments.
        assert_eq!(
            parse_text_tool_call(r#"{"name": "web_search"}"#, OFFERED).map(|call| call.args),
            Some(json!({}))
        );
    }

    #[test]
    fn only_json_looking_starts_are_held_back() {
        assert!(may_be_text_tool_call(""));
        assert!(may_be_text_tool_call("  {\"na"));
        assert!(!may_be_text_tool_call("{x} is a set"));
        assert!(may_be_text_tool_call("```"));
        assert!(may_be_text_tool_call("```json\n{"));
        assert!(may_be_text_tool_call("<tool_"));
        assert!(!may_be_text_tool_call("Rust is"));
        // Code is released at once, so a coding answer keeps streaming.
        assert!(!may_be_text_tool_call("```rust\nfn main() {}"));
        assert!(!may_be_text_tool_call(&format!("{{{}", "x".repeat(5_000))));
    }

    #[test]
    fn a_short_lead_in_before_a_trailing_call_is_split_off() {
        let text = "To find the latest release, I'll ask the WebSearchAgent.\n\n\
            {\"name\": \"WebSearchAgent\", \"arguments\": {\"request\": \"latest Rust\"}}";
        let (lead_in, call) = split_text_tool_call(text, &OFFERED).expect("trailing call");
        assert_eq!(
            text[..lead_in].trim(),
            "To find the latest release, I'll ask the WebSearchAgent."
        );
        assert_eq!(call.args, json!({ "request": "latest Rust" }));
        assert_eq!(text_tool_call_start(text, 0), Some(lead_in));

        // JSON in the middle of an answer, or after a long answer, is not a call.
        let middle = "Example:\n{\"name\": \"web_search\"}\nThat is the shape.";
        assert_eq!(split_text_tool_call(middle, &OFFERED), None);
        let long = format!("{}\n{{\"name\": \"web_search\"}}", "word ".repeat(200));
        assert_eq!(split_text_tool_call(&long, &OFFERED), None);
        assert_eq!(text_tool_call_start("Plain answer.\nSecond line.", 0), None);
        assert_eq!(text_tool_call_start("Use {x} here.\n{x} again", 0), None);
    }
}
