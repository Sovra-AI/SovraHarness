//! Closed Graph v1.0 request mapping. No model-supplied URLs or arbitrary writes.

use reqwest::Method;
use serde_json::{Value, json};
use sovra_core::connected_text::{clip_text, html_to_text};
use sovra_core::microsoft::{MicrosoftError, MicrosoftOutput, MicrosoftRequest, UNTRUSTED};

use crate::http;

const TOP: usize = 20;
const OUTPUT_CHARS: usize = 40_000;

struct Request {
    method: Method,
    url: url::Url,
    body: Option<Value>,
}

fn endpoint(base: &url::Url, segments: &[&str]) -> url::Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("Graph URL has path segments")
        .pop_if_empty()
        .extend(segments);
    url
}

fn list(base: &url::Url, segments: &[&str], select: &str) -> Request {
    let mut url = endpoint(base, segments);
    url.query_pairs_mut().append_pair("$top", &TOP.to_string());
    if !select.is_empty() {
        url.query_pairs_mut().append_pair("$select", select);
    }
    Request {
        method: Method::GET,
        url,
        body: None,
    }
}

fn get(base: &url::Url, segments: &[&str], select: &str) -> Request {
    let mut url = endpoint(base, segments);
    if !select.is_empty() {
        url.query_pairs_mut().append_pair("$select", select);
    }
    Request {
        method: Method::GET,
        url,
        body: None,
    }
}

fn post(base: &url::Url, segments: &[&str], body: Value) -> Request {
    Request {
        method: Method::POST,
        url: endpoint(base, segments),
        body: Some(body),
    }
}

fn file_search(base: &url::Url, prefix: &[&str], query: &str) -> Request {
    let function = format!("search(q='{}')", query.replace('\'', "''"));
    let mut parts = prefix.to_vec();
    parts.extend([
        "root",
        if query.is_empty() {
            "children"
        } else {
            &function
        },
    ]);
    list(
        base,
        &parts,
        "id,name,size,webUrl,file,folder,parentReference,lastModifiedDateTime",
    )
}

fn request(base: &url::Url, request: &MicrosoftRequest) -> Result<Request, MicrosoftError> {
    let result = match request {
        MicrosoftRequest::SearchMail { query } => {
            let mut request = list(
                base,
                &["me", "messages"],
                "id,subject,from,toRecipients,receivedDateTime,isRead,bodyPreview,webLink",
            );
            if query.is_empty() {
                request
                    .url
                    .query_pairs_mut()
                    .append_pair("$orderby", "receivedDateTime desc");
            } else {
                request
                    .url
                    .query_pairs_mut()
                    .append_pair("$search", &json!(query).to_string());
            }
            request
        }
        MicrosoftRequest::ReadMail { message_id } => get(
            base,
            &["me", "messages", message_id.as_str()],
            "id,subject,from,toRecipients,receivedDateTime,body,webLink",
        ),
        MicrosoftRequest::DraftMail { to, subject, body } => post(
            base,
            &["me", "messages"],
            json!({
                "subject": subject, "body": {"contentType":"Text", "content":body},
                "toRecipients":[{"emailAddress":{"address":to}}]
            }),
        ),
        MicrosoftRequest::ListEvents { from, to } => {
            let start = timestamp(from)?;
            let end = timestamp(to)?;
            if end <= start || end - start > time::Duration::days(366) {
                return Err(date_error());
            }
            let mut request = list(
                base,
                &["me", "calendarView"],
                "id,subject,start,end,location,bodyPreview,webLink,isAllDay",
            );
            request
                .url
                .query_pairs_mut()
                .append_pair("startDateTime", from)
                .append_pair("endDateTime", to)
                .append_pair("$orderby", "start/dateTime");
            request
        }
        MicrosoftRequest::AddEvent {
            subject,
            start,
            end,
            time_zone,
        } => {
            // Require offset-bearing timestamps, then normalize to UTC. This avoids
            // accepting ambiguous local clock times around daylight-saving changes.
            let start = timestamp(start)?;
            let end = timestamp(end)?;
            if end <= start || end - start > time::Duration::days(366) {
                return Err(date_error());
            }
            if time_zone != "UTC" {
                return Err(MicrosoftError::Failed(
                    "Use offset-bearing event times and time_zone UTC.".into(),
                ));
            }
            let format = time::format_description::well_known::Rfc3339;
            let date = |value: time::OffsetDateTime| {
                value
                    .to_offset(time::UtcOffset::UTC)
                    .format(&format)
                    .map_err(|_| date_error())
            };
            post(
                base,
                &["me", "events"],
                json!({
                    "subject":subject,
                    "start":{"dateTime":date(start)?,"timeZone":"UTC"},
                    "end":{"dateTime":date(end)?,"timeZone":"UTC"}
                }),
            )
        }
        MicrosoftRequest::SearchFiles { query } => file_search(base, &["me", "drive"], query),
        MicrosoftRequest::SearchContacts { query } => {
            let mut request = list(
                base,
                &["me", "contacts"],
                "id,displayName,emailAddresses,businessPhones,mobilePhone",
            );
            if !query.is_empty() {
                request.url.query_pairs_mut().append_pair(
                    "$filter",
                    &format!("startswith(displayName,'{}')", query.replace('\'', "''")),
                );
            }
            request
        }
        MicrosoftRequest::ListTaskLists => list(
            base,
            &["me", "todo", "lists"],
            "id,displayName,wellknownListName",
        ),
        MicrosoftRequest::ListTasks { list_id } => list(
            base,
            &["me", "todo", "lists", list_id.as_str(), "tasks"],
            "id,title,status,dueDateTime,body",
        ),
        MicrosoftRequest::AddTask { list_id, title } => post(
            base,
            &["me", "todo", "lists", list_id.as_str(), "tasks"],
            json!({"title":title}),
        ),
        MicrosoftRequest::ListNotes => list(
            base,
            &["me", "onenote", "pages"],
            "id,title,lastModifiedDateTime,links",
        ),
        MicrosoftRequest::ReadNote { page_id } => get(
            base,
            &["me", "onenote", "pages", page_id.as_str(), "content"],
            "",
        ),
        MicrosoftRequest::SearchSites { query } => {
            let mut request = get(base, &["sites"], "id,displayName,webUrl,description");
            request.url.query_pairs_mut().append_pair("search", query);
            request
        }
        MicrosoftRequest::ListSiteDrives { site_id } => get(
            base,
            &["sites", site_id.as_str(), "drives"],
            "id,name,webUrl,driveType",
        ),
        MicrosoftRequest::SearchSiteFiles { drive_id, query } => {
            file_search(base, &["drives", drive_id.as_str()], query)
        }
        MicrosoftRequest::ListChats => list(base, &["me", "chats"], ""),
        MicrosoftRequest::ReadChat { chat_id } => {
            list(base, &["chats", chat_id.as_str(), "messages"], "")
        }
        MicrosoftRequest::ListTeams => get(base, &["me", "joinedTeams"], ""),
        MicrosoftRequest::ListChannels { team_id } => get(
            base,
            &["teams", team_id.as_str(), "channels"],
            "id,displayName,description,webUrl",
        ),
        MicrosoftRequest::ReadChannel {
            team_id,
            channel_id,
        } => list(
            base,
            &[
                "teams",
                team_id.as_str(),
                "channels",
                channel_id.as_str(),
                "messages",
            ],
            "",
        ),
        MicrosoftRequest::ReadChannelReplies {
            team_id,
            channel_id,
            message_id,
        } => list(
            base,
            &[
                "teams",
                team_id.as_str(),
                "channels",
                channel_id.as_str(),
                "messages",
                message_id.as_str(),
                "replies",
            ],
            "",
        ),
        // Planner doesn't document $top here; bound the response and output locally.
        MicrosoftRequest::ListPlannerTasks => get(base, &["me", "planner", "tasks"], ""),
        MicrosoftRequest::ReadFile { item_id } => get(
            base,
            &["me", "drive", "items", item_id.as_str()],
            "id,name,size,file",
        ),
        MicrosoftRequest::ReadSiteFile { drive_id, item_id } => get(
            base,
            &["drives", drive_id.as_str(), "items", item_id.as_str()],
            "id,name,size,file",
        ),
    };
    Ok(result)
}

fn timestamp(value: &str) -> Result<time::OffsetDateTime, MicrosoftError> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| date_error())
}

fn date_error() -> MicrosoftError {
    MicrosoftError::Failed("Use start and end dates with a time and UTC offset, with the end after the start and within one year.".into())
}

pub(crate) async fn call(
    client: &reqwest::Client,
    base: &url::Url,
    access: &str,
    action: &MicrosoftRequest,
) -> Result<MicrosoftOutput, MicrosoftError> {
    let spec = request(base, action)?;
    let mut builder = client
        .request(spec.method, spec.url.clone())
        .bearer_auth(access)
        .header("Prefer", "outlook.body-content-type=\"text\"");
    if let Some(body) = spec.body {
        builder = builder.json(&body);
    }
    let response = builder.send().await.map_err(|_| http::offline())?;
    if matches!(action, MicrosoftRequest::ReadNote { .. }) {
        http::status(&response)?;
        let bytes = http::bytes(response, http::JSON_LIMIT).await?;
        let text = html_to_text(&String::from_utf8_lossy(&bytes));
        return Ok(MicrosoftOutput::Data(
            json!({"untrusted":UNTRUSTED, "text":clip_text(&text, OUTPUT_CHARS), "truncated":text.chars().count() > OUTPUT_CHARS}),
        ));
    }
    let value: Value = http::json(response).await?;
    if matches!(
        action,
        MicrosoftRequest::ReadFile { .. } | MicrosoftRequest::ReadSiteFile { .. }
    ) {
        return download(client, spec.url, access, &value).await;
    }
    let more = value.get("@odata.nextLink").is_some()
        || value
            .get("value")
            .and_then(Value::as_array)
            .is_some_and(|rows| rows.len() > TOP);
    let mut budget = OUTPUT_CHARS;
    let data = compact(value, &mut budget, 0);
    Ok(MicrosoftOutput::Data(
        json!({"untrusted":UNTRUSTED, "data":data, "more_available":more, "text_truncated":budget == 0}),
    ))
}

fn compact(value: Value, budget: &mut usize, depth: usize) -> Value {
    if depth > 8 || *budget == 0 {
        return Value::Null;
    }
    match value {
        Value::String(text) => {
            let used = text.chars().count().min(*budget);
            let clipped = clip_text(&text, *budget);
            *budget -= used;
            Value::String(clipped)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(TOP)
                .map(|item| compact(item, budget, depth + 1))
                .collect(),
        ),
        Value::Object(mut fields) => {
            if fields
                .get("contentType")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("html"))
            {
                if let Some(Value::String(html)) = fields.get_mut("content") {
                    *html = html_to_text(html);
                }
                fields.insert("contentType".into(), json!("text"));
            }
            Value::Object(
                fields
                    .into_iter()
                    .filter(|(key, _)| {
                        !key.starts_with('@') && key != "contentUrl" && key != "downloadUrl"
                    })
                    .map(|(key, value)| (key, compact(value, budget, depth + 1)))
                    .collect(),
            )
        }
        other => other,
    }
}

async fn download(
    client: &reqwest::Client,
    mut item_url: url::Url,
    access: &str,
    metadata: &Value,
) -> Result<MicrosoftOutput, MicrosoftError> {
    let limit = usize::try_from(sovra_core::files::MAX_FILE_BYTES).expect("20 MB fits usize");
    if metadata.get("file").is_none()
        || metadata["size"]
            .as_u64()
            .is_none_or(|size| size > limit as u64)
    {
        return Err(MicrosoftError::Failed(
            "Choose a file of at most 20 MB.".into(),
        ));
    }
    let name = metadata["name"]
        .as_str()
        .ok_or_else(|| MicrosoftError::Failed("Microsoft didn't return the file name.".into()))?
        .to_owned();
    item_url.set_query(None);
    item_url
        .path_segments_mut()
        .expect("Graph path")
        .push("content");
    let mut response = client
        .get(item_url)
        .bearer_auth(access)
        .send()
        .await
        .map_err(|_| http::offline())?;
    // Only the initial Graph request carries account access. Redirects are
    // preauthorized, short-lived links, never model-visible.
    for _ in 0..5 {
        if !response.status().is_redirection() {
            break;
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| url::Url::parse(value).ok())
            .filter(download_host)
            .ok_or_else(|| {
                MicrosoftError::Failed(
                    "Microsoft returned an unsupported file download address.".into(),
                )
            })?;
        response = client
            .get(location)
            .send()
            .await
            .map_err(|_| http::offline())?;
    }
    http::status(&response)?;
    let bytes = http::bytes(response, limit).await?;
    Ok(MicrosoftOutput::File { name, bytes })
}

fn download_host(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            [
                "sharepoint.com",
                "sharepoint-df.com",
                "1drv.com",
                "onedrive.com",
                "storage.live.com",
            ]
            .iter()
            .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        })
}

#[cfg(test)]
mod tests;
