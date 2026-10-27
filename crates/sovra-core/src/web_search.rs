//! Pure `SearXNG` result parsing, URL canonicalization, and stable ranking.
//!
//! No HTTP. Adapters fetch JSON and pass it here so duplicate URLs collapse the
//! same way in tests and in the agent.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Specialist id the router uses for an explicit web-search turn.
pub const WEB_SEARCH_AGENT_ID: &str = "WebSearchAgent";

/// One source the agent is allowed to cite. `title`, `url`, and snippet come from the payload only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// One query's raw `SearXNG` body, or the fetch error that replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryOutcome {
    pub query: String,
    pub body: Result<String, String>,
}

/// Deduped hits in first-seen order, plus per-query failures that did not erase them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedSearch {
    #[serde(rename = "results")]
    pub hits: Vec<SearchHit>,
    pub errors: Vec<String>,
}

/// True when the user asked for a web search, even if the topic contains words
/// such as "rust" or "code" that would otherwise select another specialist.
pub fn is_explicit_web_search(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("search the web")
        || lower.contains("search the internet")
        || lower.contains("web search for")
}

/// Canonical `http`/`https` URL used as the dedupe key.
///
/// Scheme and host are lowercased, the fragment and default port are dropped,
/// and a trailing slash is removed. Returns `None` when `raw` is not an HTTP URL,
/// so a malformed row cannot become a hit.
pub fn canonical_http_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 2_000 {
        return None;
    }
    let (scheme, rest) = split_http_scheme(trimmed)?;
    if rest.is_empty() || rest.chars().any(char::is_whitespace) {
        return None;
    }
    let without_fragment = rest.split_once('#').map_or(rest, |(head, _)| head);
    if without_fragment.is_empty() {
        return None;
    }
    let (authority, path_query) = match without_fragment.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (without_fragment, String::new()),
    };
    // Credentials are not part of a citable source.
    let hostport = authority
        .rsplit_once('@')
        .map_or(authority, |(_, hostport)| hostport);
    let (host, port) = split_host_port(hostport)?;
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let port_suffix = match (scheme, port) {
        (_, None) | ("http", Some(80)) | ("https", Some(443)) => String::new(),
        (_, Some(port)) => format!(":{port}"),
    };
    let path_query = normalize_path_query(&path_query);
    Some(format!("{scheme}://{host}{port_suffix}{path_query}"))
}

/// Parses one `SearXNG` JSON document (`results` of `title`, `url`, `content` or `snippet`).
///
/// Rows without an HTTP URL are skipped. Duplicate URLs keep the first row.
/// The document must be JSON; a non-object or invalid document is an error so the
/// caller can keep sibling queries.
pub fn parse_and_rank_searxng(body: &str) -> Result<Vec<SearchHit>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("invalid SearXNG JSON: {error}"))?;
    let Some(object) = value.as_object() else {
        return Err("SearXNG response is not a JSON object".to_owned());
    };
    let Some(results) = object.get("results") else {
        return Ok(Vec::new());
    };
    let Some(results) = results.as_array() else {
        return Err("SearXNG `results` is not an array".to_owned());
    };
    Ok(rank_rows(results))
}

/// Merges query outcomes in input order. A failed or unreadable query is recorded
/// and does not remove hits from the queries that succeeded.
pub fn rank_search_outcomes(outcomes: &[QueryOutcome]) -> RankedSearch {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    let mut errors = Vec::new();
    for outcome in outcomes {
        match &outcome.body {
            Ok(body) => match parse_and_rank_searxng(body) {
                Ok(parsed) => {
                    for hit in parsed {
                        if seen.insert(hit.url.clone()) {
                            hits.push(hit);
                        }
                    }
                }
                Err(error) => errors.push(format!("{}: {error}", outcome.query)),
            },
            Err(error) => errors.push(format!("{}: {error}", outcome.query)),
        }
    }
    RankedSearch { hits, errors }
}

fn rank_rows(results: &[serde_json::Value]) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for row in results {
        let Some(object) = row.as_object() else {
            continue;
        };
        let Some(raw_url) = object.get("url").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(url) = canonical_http_url(raw_url) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = object
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let snippet = object
            .get("content")
            .or_else(|| object.get("snippet"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        hits.push(SearchHit {
            title,
            url,
            snippet,
        });
    }
    hits
}

fn split_http_scheme(raw: &str) -> Option<(&'static str, &str)> {
    let (scheme, rest) = raw.split_once("://")?;
    if scheme.eq_ignore_ascii_case("https") {
        Some(("https", rest))
    } else if scheme.eq_ignore_ascii_case("http") {
        Some(("http", rest))
    } else {
        None
    }
}

fn split_host_port(hostport: &str) -> Option<(&str, Option<u16>)> {
    if hostport.is_empty() {
        return None;
    }
    if let Some(rest) = hostport.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        let port = if let Some(port) = after.strip_prefix(':') {
            if !after.starts_with(':') || port.is_empty() {
                return None;
            }
            Some(parse_port(port)?)
        } else if after.is_empty() {
            None
        } else {
            return None;
        };
        return Some((host, port));
    }
    match hostport.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty()
                && !host.contains(':')
                && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            Some((host, Some(parse_port(port)?)))
        }
        Some(_) => None,
        None => Some((hostport, None)),
    }
}

fn parse_port(raw: &str) -> Option<u16> {
    raw.parse::<u16>().ok().filter(|port| *port > 0)
}

fn normalize_path_query(path_query: &str) -> String {
    if path_query.is_empty() || path_query == "/" {
        return String::new();
    }
    let (path, query) = path_query
        .split_once('?')
        .map_or((path_query, None), |(path, query)| (path, Some(query)));
    let path = path.trim_end_matches('/');
    match query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_phrase_matches_search_the_web_and_not_a_greeting() {
        assert!(is_explicit_web_search(
            "Please search the web for rust async traits"
        ));
        assert!(is_explicit_web_search("WEB SEARCH FOR rust release notes"));
        assert!(!is_explicit_web_search("hello"));
        assert!(!is_explicit_web_search("debug this rust code"));
    }

    #[test]
    fn parse_collapses_duplicate_urls_skips_malformed_rows_and_invents_nothing() {
        let body = r#"{
            "query": "rust",
            "results": [
                {"title": "Rust", "url": "https://Example.com/rust/", "content": "notes"},
                {"title": "Other", "url": "not-a-row"},
                {"title": "Dup", "url": "https://example.com/rust#section", "content": "second"},
                {"title": "Port", "url": "https://example.com:443/rust", "content": "port"},
                {"url": "https://example.com/other", "content": "no title"},
                {"title": "Missing url", "content": "nope"},
                "bad-row",
                {"title": "Ok", "url": "https://example.com/ok", "snippet": "via snippet"}
            ]
        }"#;
        let hits = parse_and_rank_searxng(body).expect("fixture is valid JSON");
        let urls: Vec<&str> = hits.iter().map(|hit| hit.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://example.com/rust",
                "https://example.com/other",
                "https://example.com/ok"
            ]
        );
        assert_eq!(hits[0].title, "Rust");
        assert_eq!(hits[0].snippet, "notes");
        assert_eq!(hits[1].title, "");
        assert_eq!(hits[2].snippet, "via snippet");
        let allowed = [
            "https://Example.com/rust/",
            "https://example.com/rust#section",
            "https://example.com:443/rust",
            "https://example.com/other",
            "https://example.com/ok",
        ];
        let allowed: Vec<String> = allowed
            .into_iter()
            .map(|raw| canonical_http_url(raw).expect("fixture url"))
            .collect();
        for hit in &hits {
            assert!(
                allowed.iter().any(|url| url == &hit.url),
                "invented url {}",
                hit.url
            );
        }
    }

    #[test]
    fn failed_query_does_not_drop_sibling_hits() {
        let good =
            r#"{"results":[{"title":"Kept","url":"https://kept.example/a","content":"yes"}]}"#;
        let ranked = rank_search_outcomes(&[
            QueryOutcome {
                query: "good".to_owned(),
                body: Ok(good.to_owned()),
            },
            QueryOutcome {
                query: "down".to_owned(),
                body: Err("connection refused".to_owned()),
            },
            QueryOutcome {
                query: "junk".to_owned(),
                body: Ok("not-json".to_owned()),
            },
        ]);
        assert_eq!(ranked.hits.len(), 1);
        assert_eq!(ranked.hits[0].url, "https://kept.example/a");
        assert_eq!(ranked.hits[0].title, "Kept");
        assert_eq!(ranked.errors.len(), 2);
        assert!(ranked.errors.iter().any(|error| error.contains("down")));
        assert!(ranked.errors.iter().any(|error| error.contains("junk")));
        assert!(
            ranked
                .hits
                .iter()
                .all(|hit| hit.url.contains("kept.example"))
        );
    }

    #[test]
    fn canonical_url_rejects_non_http_and_blank_values() {
        assert_eq!(canonical_http_url("javascript:alert(1)"), None);
        assert_eq!(canonical_http_url("ftp://example.com/a"), None);
        assert_eq!(canonical_http_url("   "), None);
        assert_eq!(canonical_http_url("https://"), None);
    }
}
