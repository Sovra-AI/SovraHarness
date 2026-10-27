//! Gmail message shapes: building a draft (RFC 2822, base64url) and reading the
//! readable text out of a `format=full` payload.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use std::fmt::Write as _;

use serde_json::Value;

/// The original message a draft replies to.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReplyTo {
    pub(crate) message_id: String,
    pub(crate) references: String,
    pub(crate) subject: String,
}

/// Header values must stay on one line, or a model could add its own headers (Bcc).
fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// RFC 2047 for non-ASCII subjects.
fn encode_header(value: &str) -> String {
    if value.is_ascii() {
        value.to_owned()
    } else {
        format!("=?UTF-8?B?{}?=", STANDARD.encode(value))
    }
}

/// A plain address list, or `None` if any entry is not an email address.
pub(crate) fn recipients(to: &[String]) -> Option<String> {
    let addresses: Vec<String> = to
        .iter()
        .map(|a| one_line(a))
        .filter(|a| !a.is_empty())
        .collect();
    let valid = addresses.iter().all(|address| {
        let bare = address
            .rsplit_once('<')
            .map_or(address.as_str(), |(_, rest)| rest.trim_end_matches('>'));
        bare.split_once('@')
            .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'))
            && !bare.contains(char::is_whitespace)
            && !bare.contains(',')
    });
    (valid && !addresses.is_empty()).then(|| addresses.join(", "))
}

/// The base64url `raw` field for `drafts.create`.
pub(crate) fn raw_draft(to: &str, subject: &str, body: &str, reply: Option<&ReplyTo>) -> String {
    let mut subject = one_line(subject);
    if let Some(reply) = reply {
        if subject.is_empty() {
            subject = one_line(&reply.subject);
        }
        if !subject.to_lowercase().starts_with("re:") {
            subject = format!("Re: {subject}");
        }
    }
    let mut message = format!(
        "To: {to}\r\nSubject: {}\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=\"UTF-8\"\r\nContent-Transfer-Encoding: base64\r\n",
        encode_header(&subject)
    );
    if let Some(reply) = reply.filter(|r| !r.message_id.is_empty()) {
        let id = one_line(&reply.message_id);
        let references = one_line(&format!("{} {id}", reply.references));
        let _ = write!(message, "In-Reply-To: {id}\r\nReferences: {references}\r\n");
    }
    message.push_str("\r\n");
    let encoded = STANDARD.encode(body.replace("\r\n", "\n").replace('\n', "\r\n"));
    for line in encoded.as_bytes().chunks(76) {
        message.push_str(&String::from_utf8_lossy(line));
        message.push_str("\r\n");
    }
    URL_SAFE_NO_PAD.encode(message)
}

pub(crate) fn header<'a>(payload: &'a Value, name: &str) -> &'a str {
    payload["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h["value"].as_str())
        .unwrap_or_default()
}

fn decode_part(data: &str) -> Option<String> {
    let bytes = URL_SAFE_NO_PAD.decode(data.trim_end_matches('=')).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Readable text: the first `text/plain` part, else the first `text/html` part
/// turned into text. Attachments are skipped.
pub(crate) fn body_text(payload: &Value) -> String {
    fn find(part: &Value, mime: &str) -> Option<String> {
        let is_attachment = part["filename"].as_str().is_some_and(|f| !f.is_empty());
        if !is_attachment
            && part["mimeType"].as_str() == Some(mime)
            && let Some(text) = part["body"]["data"].as_str().and_then(decode_part)
        {
            return Some(text);
        }
        part["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|child| find(child, mime))
    }
    find(payload, "text/plain")
        .or_else(|| find(payload, "text/html").map(|html| sovra_core::google::html_to_text(&html)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn decoded(raw: &str) -> String {
        String::from_utf8(URL_SAFE_NO_PAD.decode(raw).expect("base64url")).expect("utf-8")
    }

    #[test]
    fn a_model_cannot_smuggle_extra_headers_into_a_draft() {
        let raw = raw_draft("a@b.co", "Hi\r\nBcc: evil@x.co", "Body", None);
        let message = decoded(&raw);
        assert!(!message.contains("\r\nBcc:"));
        assert!(message.contains("Subject: Hi  Bcc: evil@x.co\r\n"));
    }

    #[test]
    fn recipients_must_be_email_addresses() {
        assert_eq!(
            recipients(&["Anna <anna@example.com>".into(), "bo@example.org".into()]).as_deref(),
            Some("Anna <anna@example.com>, bo@example.org")
        );
        assert_eq!(recipients(&["not an address".into()]), None);
        assert_eq!(recipients(&["a@b.co, c@d.co".into()]), None);
        assert_eq!(recipients(&[]), None);
    }

    #[test]
    fn replies_thread_correctly_and_encode_unicode_subjects() {
        let reply = ReplyTo {
            message_id: "<m1@mail>".into(),
            references: "<m0@mail>".into(),
            subject: "Café".into(),
        };
        let message = decoded(&raw_draft("a@b.co", "", "Merci\nÀ bientôt", Some(&reply)));
        assert!(message.contains("In-Reply-To: <m1@mail>\r\n"));
        assert!(message.contains("References: <m0@mail> <m1@mail>\r\n"));
        assert!(message.contains(&format!(
            "Subject: =?UTF-8?B?{}?=",
            STANDARD.encode("Re: Café")
        )));
        let body = message
            .split("\r\n\r\n")
            .nth(1)
            .expect("body")
            .replace("\r\n", "");
        assert_eq!(
            String::from_utf8(STANDARD.decode(body).expect("base64")).expect("utf-8"),
            "Merci\r\nÀ bientôt"
        );
    }

    #[test]
    fn body_prefers_plain_text_and_falls_back_to_html() {
        let encode = |s: &str| URL_SAFE_NO_PAD.encode(s);
        let payload = json!({
            "mimeType": "multipart/alternative",
            "headers": [{"name": "Subject", "value": "Lunch"}],
            "parts": [
                {"mimeType": "text/html", "body": {"data": encode("<p>html</p>")}},
                {"mimeType": "text/plain", "body": {"data": encode("plain")}},
                {"mimeType": "text/plain", "filename": "a.txt", "body": {"data": encode("file")}}
            ]
        });
        assert_eq!(body_text(&payload), "plain");
        assert_eq!(header(&payload, "subject"), "Lunch");
        let html_only =
            json!({"mimeType": "text/html", "body": {"data": encode("<b>Hi</b> there")}});
        assert_eq!(body_text(&html_only), "Hi there");
    }
}
