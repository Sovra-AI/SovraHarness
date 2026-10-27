//! Pure web search result parsing, URL canonicalization, and rank fusion.
//!
//! No HTTP. Adapters fetch each search source's JSON and pass it here, so the
//! same rows collapse the same way in tests and in the agent. Every source is
//! free, keyless and open: Mwmbl and Wikipedia are built in; a self-hosted
//! `SearXNG` is an optional extra.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::memory::RRF_K;

/// Specialist id the router uses for an explicit web-search turn.
pub const WEB_SEARCH_AGENT_ID: &str = "WebSearchAgent";

/// Most hits one tool call returns after fusion. Keeps the model's context small.
pub const MAX_FUSED_HITS: usize = 12;

/// Only the top of each source's list counts; deep tails are mostly noise.
const MAX_HITS_PER_LIST: usize = 10;
const MAX_SNIPPET_CHARS: usize = 320;
const MAX_TITLE_CHARS: usize = 200;

/// Query parameters that only track the click and never change the page.
const TRACKING_PARAMS: &[&str] = &["fbclid", "gclid", "dclid", "msclkid", "mc_cid", "mc_eid"];

/// One source the agent is allowed to cite. `title`, `url`, and snippet come from the payload only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// Search sources that returned this URL, in first-seen order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

/// One query's raw `SearXNG`-shaped body, or the fetch error that replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryOutcome {
    pub query: String,
    pub body: Result<String, String>,
}

/// One search source's parsed hits for one query, or why it produced none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOutcome {
    pub engine: String,
    pub query: String,
    pub hits: Result<Vec<SearchHit>, String>,
}

/// Fused hits, best first, plus per-source failures that did not erase them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedSearch {
    #[serde(rename = "results")]
    pub hits: Vec<SearchHit>,
    pub errors: Vec<String>,
}

/// Readable text of one opened web page. `url` is where the page ended up after
/// redirects, so a citation points at what was actually read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebPage {
    pub url: String,
    pub title: String,
    pub text: String,
    /// True when the page was longer than the reading budget and was cut.
    pub truncated: bool,
}

/// True when the user asked for a web search, even if the topic contains words
/// such as "rust" or "code" that would otherwise select another specialist.
///
/// Deliberately narrow: an explicit request also drops unrelated attachments, so
/// "search for the bug in this file" must not match.
pub fn is_explicit_web_search(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        "search the web",
        "search the internet",
        "search the net",
        "web search for",
        "search online",
        "look online",
        "look it up online",
        "look up online",
        "find online",
        "google it",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

/// Canonical `http`/`https` URL used as the dedupe key.
///
/// Scheme and host are lowercased; the fragment, default port, click-tracking
/// parameters and a trailing slash are dropped. Returns `None` when `raw` is not
/// an HTTP URL, so a malformed row cannot become a hit.
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
    let (authority, path_query) = match without_fragment.find(['/', '?']) {
        Some(split) => (
            &without_fragment[..split],
            if without_fragment[split..].starts_with('?') {
                format!("/{}", &without_fragment[split..])
            } else {
                without_fragment[split..].to_owned()
            },
        ),
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
    Ok(dedupe_rows(results.iter().filter_map(|row| {
        let object = row.as_object()?;
        let url = object.get("url")?.as_str()?;
        let title = object.get("title").and_then(serde_json::Value::as_str);
        let snippet = object
            .get("content")
            .or_else(|| object.get("snippet"))
            .and_then(serde_json::Value::as_str);
        Some((
            url,
            title.unwrap_or("").to_owned(),
            snippet.unwrap_or("").to_owned(),
        ))
    })))
}

/// Parses the Mwmbl search API (`[{url, title: [{value}], extract: [{value}]}]`).
///
/// Mwmbl splits titles and extracts into highlighted segments; they are joined
/// back into plain text so no markup reaches the model.
pub fn parse_mwmbl(body: &str) -> Result<Vec<SearchHit>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("invalid Mwmbl JSON: {error}"))?;
    let Some(rows) = value.as_array() else {
        return Err("Mwmbl response is not a JSON array".to_owned());
    };
    Ok(dedupe_rows(rows.iter().filter_map(|row| {
        let object = row.as_object()?;
        let url = object.get("url")?.as_str()?;
        let title = join_segments(object.get("title"));
        let snippet = join_segments(object.get("extract"));
        Some((url, title, snippet))
    })))
}

/// Parses `MediaWiki` `list=search` JSON into article links on `site` (e.g. `en.wikipedia.org`).
///
/// `MediaWiki` snippets carry `<span class="searchmatch">` highlights; they are
/// stripped to plain text.
pub fn parse_wikipedia(body: &str, site: &str) -> Result<Vec<SearchHit>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("invalid Wikipedia JSON: {error}"))?;
    if let Some(error) = value.get("error") {
        let info = error
            .get("info")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("Wikipedia error: {info}"));
    }
    let Some(rows) = value
        .get("query")
        .and_then(|query| query.get("search"))
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    Ok(dedupe_rows(rows.iter().filter_map(|row| {
        let title = row.get("title")?.as_str()?.trim();
        if title.is_empty() {
            return None;
        }
        let snippet = row
            .get("snippet")
            .and_then(serde_json::Value::as_str)
            .map(strip_html)
            .unwrap_or_default();
        Some((
            wikipedia_article_url(site, title),
            title.to_owned(),
            snippet,
        ))
    })))
}

/// `https://{site}/wiki/{Title_With_Underscores}`, percent-encoding everything
/// that is not unreserved or a character `MediaWiki` keeps literal in titles.
pub fn wikipedia_article_url(site: &str, title: &str) -> String {
    let mut path = String::with_capacity(title.len());
    for byte in title.trim().replace(' ', "_").bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~()!*',:@".contains(&byte) {
            path.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            path.push('%');
            path.push(char::from(HEX[usize::from(byte >> 4)]));
            path.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    format!("https://{site}/wiki/{path}")
}

/// Merges query outcomes from one `SearXNG`-shaped source. A failed or unreadable
/// query is recorded and does not remove hits from the queries that succeeded.
pub fn rank_search_outcomes(outcomes: &[QueryOutcome]) -> RankedSearch {
    let engine_outcomes: Vec<EngineOutcome> = outcomes
        .iter()
        .map(|outcome| EngineOutcome {
            engine: "searxng".to_owned(),
            query: outcome.query.clone(),
            hits: outcome
                .body
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|body| parse_and_rank_searxng(body)),
        })
        .collect();
    fuse_engine_outcomes(&engine_outcomes, MAX_FUSED_HITS)
}

/// Reciprocal-rank fusion across every (source, query) list.
///
/// A URL found by several sources or queries rises; ties keep first-seen order,
/// so the result is deterministic for a given input order. Failures become
/// `errors` and never remove hits that other lists found.
pub fn fuse_engine_outcomes(outcomes: &[EngineOutcome], limit: usize) -> RankedSearch {
    let mut order: Vec<String> = Vec::new();
    let mut fused: HashMap<String, (SearchHit, f32)> = HashMap::new();
    let mut errors = Vec::new();
    for outcome in outcomes {
        let hits = match &outcome.hits {
            Ok(hits) => hits,
            Err(error) => {
                errors.push(format!("{} ({}): {error}", outcome.query, outcome.engine));
                continue;
            }
        };
        let mut seen_in_list = HashSet::new();
        for (rank, hit) in hits.iter().take(MAX_HITS_PER_LIST).enumerate() {
            let Some(url) = canonical_http_url(&hit.url) else {
                continue;
            };
            if !seen_in_list.insert(url.clone()) {
                continue;
            }
            #[expect(
                clippy::cast_precision_loss,
                reason = "ranks are at most MAX_HITS_PER_LIST"
            )]
            let contribution = 1.0 / (RRF_K + rank as f32);
            if let Some((existing, score)) = fused.get_mut(&url) {
                *score += contribution;
                merge_hit(existing, hit, &outcome.engine);
            } else {
                order.push(url.clone());
                let mut fresh = clean_hit(hit, url.clone());
                fresh.sources = vec![outcome.engine.clone()];
                fused.insert(url, (fresh, contribution));
            }
        }
    }
    let mut ranked: Vec<(SearchHit, f32)> = order
        .into_iter()
        .filter_map(|url| fused.remove(&url))
        .collect();
    // Stable sort: equal scores keep first-seen order.
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    RankedSearch {
        hits: ranked.into_iter().take(limit).map(|(hit, _)| hit).collect(),
        errors,
    }
}

/// Plain text from a short HTML fragment: tags removed, common entities decoded,
/// whitespace collapsed. Meant for search snippets, not whole pages.
pub fn strip_html(fragment: &str) -> String {
    let mut text = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for character in fragment.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    collapse_whitespace(&decode_entities(&text))
}

/// Gregorian calendar date for days since 1970-01-01 (Howard Hinnant's algorithm),
/// plus the English weekday. Lets the search prompt state today's date without a
/// time crate in the pure core.
pub fn civil_date_from_unix_days(days: i64) -> (i64, u32, u32, &'static str) {
    const WEEKDAYS: [&str; 7] = [
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
    ];
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let weekday = WEEKDAYS[usize::try_from(days.rem_euclid(7)).unwrap_or(0)];
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
        weekday,
    )
}

fn dedupe_rows<U: AsRef<str>>(rows: impl Iterator<Item = (U, String, String)>) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for (raw_url, title, snippet) in rows {
        let Some(url) = canonical_http_url(raw_url.as_ref()) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        hits.push(SearchHit {
            title: title.trim().to_owned(),
            url,
            snippet: snippet.trim().to_owned(),
            sources: Vec::new(),
        });
    }
    hits
}

fn join_segments(value: Option<&serde_json::Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(text) = value.as_str() {
        return collapse_whitespace(text);
    }
    let mut text = String::new();
    for segment in value.as_array().into_iter().flatten() {
        if let Some(part) = segment
            .get("value")
            .and_then(serde_json::Value::as_str)
            .or_else(|| segment.as_str())
        {
            text.push_str(part);
        }
    }
    collapse_whitespace(&text)
}

fn clean_hit(hit: &SearchHit, url: String) -> SearchHit {
    SearchHit {
        title: truncate_chars(&collapse_whitespace(&hit.title), MAX_TITLE_CHARS),
        url,
        snippet: truncate_chars(&collapse_whitespace(&hit.snippet), MAX_SNIPPET_CHARS),
        sources: Vec::new(),
    }
}

/// Fills gaps from a later duplicate; the first-seen title and snippet win.
fn merge_hit(existing: &mut SearchHit, duplicate: &SearchHit, engine: &str) {
    if existing.title.is_empty() {
        existing.title = truncate_chars(&collapse_whitespace(&duplicate.title), MAX_TITLE_CHARS);
    }
    if existing.snippet.is_empty() {
        existing.snippet =
            truncate_chars(&collapse_whitespace(&duplicate.snippet), MAX_SNIPPET_CHARS);
    }
    if !existing.sources.iter().any(|source| source == engine) {
        existing.sources.push(engine.to_owned());
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max).collect();
    cut.push('…');
    cut
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        // Last, so "&amp;lt;" decodes to the literal "&lt;" and not "<".
        .replace("&amp;", "&")
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
            if port.is_empty() {
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
    let query = query.map(strip_tracking_params).unwrap_or_default();
    match (path.is_empty(), query.is_empty()) {
        (_, true) => path.to_owned(),
        (true, false) => format!("/?{query}"),
        (false, false) => format!("{path}?{query}"),
    }
}

fn strip_tracking_params(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| {
            let key = pair.split_once('=').map_or(*pair, |(key, _)| key);
            let key = key.to_ascii_lowercase();
            !key.is_empty() && !key.starts_with("utm_") && !TRACKING_PARAMS.contains(&key.as_str())
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(engine: &str, query: &str, hits: Result<Vec<SearchHit>, String>) -> EngineOutcome {
        EngineOutcome {
            engine: engine.to_owned(),
            query: query.to_owned(),
            hits,
        }
    }

    fn hit(title: &str, url: &str, snippet: &str) -> SearchHit {
        SearchHit {
            title: title.to_owned(),
            url: url.to_owned(),
            snippet: snippet.to_owned(),
            sources: Vec::new(),
        }
    }

    #[test]
    fn explicit_phrase_matches_search_the_web_and_not_a_greeting() {
        assert!(is_explicit_web_search(
            "Please search the web for rust async traits"
        ));
        assert!(is_explicit_web_search("WEB SEARCH FOR rust release notes"));
        assert!(is_explicit_web_search("can you look it up online for me"));
        assert!(is_explicit_web_search("Google it please"));
        assert!(!is_explicit_web_search("hello"));
        assert!(!is_explicit_web_search("debug this rust code"));
        // Searching inside attached files must not drop those files.
        assert!(!is_explicit_web_search("search for the bug in this file"));
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

    #[test]
    fn canonical_url_drops_click_tracking_but_keeps_real_parameters() {
        assert_eq!(
            canonical_http_url("https://news.example/a?id=7&utm_source=x&fbclid=y").as_deref(),
            Some("https://news.example/a?id=7")
        );
        assert_eq!(
            canonical_http_url("https://news.example/a?utm_medium=feed").as_deref(),
            Some("https://news.example/a")
        );
        assert_eq!(
            canonical_http_url("https://news.example?q=1").as_deref(),
            Some("https://news.example/?q=1")
        );
    }

    #[test]
    fn mwmbl_segments_join_into_plain_text() {
        let body = r#"[
            {"url": "https://rust-lang.org/",
             "title": [{"value": "Rust ", "is_bold": true}, {"value": "Language", "is_bold": false}],
             "extract": [{"value": " A language   empowering ", "is_bold": false}, {"value": "everyone", "is_bold": true}],
             "source": "mwmbl"},
            {"url": "mailto:someone@example.com", "title": [], "extract": []},
            {"url": "https://rust-lang.org", "title": [{"value": "Duplicate"}], "extract": []}
        ]"#;
        let hits = parse_mwmbl(body).expect("fixture is valid JSON");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://rust-lang.org");
        assert_eq!(hits[0].title, "Rust Language");
        assert_eq!(hits[0].snippet, "A language empowering everyone");
        assert!(parse_mwmbl(r#"{"error":"down"}"#).is_err());
    }

    #[test]
    fn wikipedia_rows_become_article_links_without_markup() {
        let body = r#"{"query":{"search":[
            {"ns":0,"title":"Rust (programming language)","pageid":1,
             "snippet":"<span class=\"searchmatch\">Rust</span> is a general-purpose &amp; safe language"},
            {"ns":0,"title":"Café au lait","pageid":2,"snippet":""}
        ]}}"#;
        let hits = parse_wikipedia(body, "en.wikipedia.org").expect("fixture is valid JSON");
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].url,
            "https://en.wikipedia.org/wiki/Rust_(programming_language)"
        );
        assert_eq!(hits[0].title, "Rust (programming language)");
        assert_eq!(hits[0].snippet, "Rust is a general-purpose & safe language");
        assert_eq!(
            hits[1].url,
            "https://en.wikipedia.org/wiki/Caf%C3%A9_au_lait"
        );
        let error = parse_wikipedia(r#"{"error":{"info":"bad request"}}"#, "en.wikipedia.org");
        assert_eq!(error, Err("Wikipedia error: bad request".to_owned()));
    }

    #[test]
    fn fusion_promotes_agreement_and_records_every_source() {
        let ranked = fuse_engine_outcomes(
            &[
                engine(
                    "mwmbl",
                    "rust",
                    Ok(vec![
                        hit("Forum", "https://users.rust-lang.org", "forum"),
                        hit("Rust", "https://rust-lang.org/", ""),
                    ]),
                ),
                engine(
                    "wikipedia",
                    "rust",
                    Ok(vec![
                        hit("Rust (lang)", "https://rust-lang.org", "official site"),
                        hit("Wiki", "https://en.wikipedia.org/wiki/Rust", "article"),
                    ]),
                ),
                engine("searxng", "rust", Err("HTTP 429".to_owned())),
            ],
            MAX_FUSED_HITS,
        );
        let urls: Vec<&str> = ranked.hits.iter().map(|hit| hit.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://rust-lang.org",
                "https://users.rust-lang.org",
                "https://en.wikipedia.org/wiki/Rust"
            ]
        );
        // First-seen title wins; the empty snippet is filled from the duplicate.
        assert_eq!(ranked.hits[0].title, "Rust");
        assert_eq!(ranked.hits[0].snippet, "official site");
        assert_eq!(ranked.hits[0].sources, ["mwmbl", "wikipedia"]);
        assert_eq!(ranked.errors, ["rust (searxng): HTTP 429"]);
    }

    #[test]
    fn fusion_caps_hits_and_snippet_length() {
        let long = "word ".repeat(200);
        let rows: Vec<SearchHit> = (0..30)
            .map(|index| hit("t", &format!("https://site.example/{index}"), &long))
            .collect();
        let ranked = fuse_engine_outcomes(&[engine("mwmbl", "q", Ok(rows))], 5);
        assert_eq!(ranked.hits.len(), 5);
        assert!(ranked.hits[0].snippet.chars().count() <= MAX_SNIPPET_CHARS + 1);
        assert!(ranked.hits[0].snippet.ends_with('…'));
    }

    #[test]
    fn strip_html_decodes_entities_once() {
        assert_eq!(strip_html("<b>A</b>&amp;lt;B&gt;"), "A &lt;B>");
        assert_eq!(strip_html("  plain\n text "), "plain text");
    }

    #[test]
    fn civil_date_matches_known_days() {
        assert_eq!(civil_date_from_unix_days(0), (1970, 1, 1, "Thursday"));
        // 2026-10-27 is 20_753 days after the epoch.
        assert_eq!(civil_date_from_unix_days(20_753), (2026, 10, 27, "Tuesday"));
        assert_eq!(civil_date_from_unix_days(11_016), (2000, 2, 29, "Tuesday"));
        assert_eq!(civil_date_from_unix_days(-1), (1969, 12, 31, "Wednesday"));
    }
}
