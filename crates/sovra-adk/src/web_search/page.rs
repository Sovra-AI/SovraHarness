//! `read_web_page`: fetch one public page and keep only its readable text.
//!
//! The model chooses the URL, and page text may try to steer it, so the fetch is
//! confined to the public internet: hosts on this machine or the local network
//! are refused before the request, on every redirect, and against the address the
//! connection actually reached. Pages are read as text only and never executed.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::LazyLock;
use std::time::Duration;

use scraper::{ElementRef, Html, Node, Selector};
use sovra_core::WebPage;

use super::sources::{USER_AGENT, describe_request_error, read_capped};

const PAGE_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_REDIRECTS: usize = 5;
/// Enough for any article; larger bodies are cut before parsing.
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
/// Reading budget handed to the model, in characters.
pub(super) const MAX_PAGE_CHARS: usize = 12_000;

/// Elements whose text is never content a reader would see.
const SKIPPED: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "canvas", "iframe", "object", "nav",
    "header", "footer", "aside", "form", "button", "select", "option", "head",
];
/// Elements that start a new line in the extracted text.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "tr",
    "table",
    "blockquote",
    "pre",
    "figure",
    "figcaption",
    "hr",
    "br",
    "address",
    "details",
    "summary",
];

static CONTENT_ROOTS: LazyLock<Vec<Selector>> = LazyLock::new(|| {
    ["article", "main", "[role=main]", "body"]
        .into_iter()
        .map(|selector| Selector::parse(selector).expect("static selector is valid CSS"))
        .collect()
});
static TITLE: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("title").expect("static selector is valid CSS"));
static HEADING: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("h1").expect("static selector is valid CSS"));

/// Client for page reads. Every redirect hop is checked against the same rules
/// as the first URL.
pub(super) fn page_client() -> Result<reqwest::Client, String> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match check_public_url(attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(reason) => attempt.error(reason),
        }
    });
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(PAGE_TIMEOUT)
        .redirect(policy)
        .build()
        .map_err(|error| error.to_string())
}

pub(super) async fn fetch_page(client: &reqwest::Client, raw_url: &str) -> Result<WebPage, String> {
    let url =
        reqwest::Url::parse(raw_url.trim()).map_err(|_| "that is not a web address".to_owned())?;
    check_public_url(&url)?;
    let response = client
        .get(url)
        .header(
            http::header::ACCEPT,
            "text/html,application/xhtml+xml,text/plain;q=0.9",
        )
        .send()
        .await
        .map_err(describe_request_error)?;
    // DNS can point a public name at a private address; refuse to read the reply.
    if let Some(address) = response.remote_addr()
        && !is_public_ip(address.ip())
    {
        return Err("that address is on this computer or a private network".to_owned());
    }
    let status = response.status();
    if !status.is_success() {
        return Err(format!("the page answered HTTP {}", status.as_u16()));
    }
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("text/html")
        .to_ascii_lowercase();
    let is_html = content_type.contains("html");
    if !is_html && !content_type.starts_with("text/plain") {
        let kind = content_type.split(';').next().unwrap_or("unknown").trim();
        return Err(format!("not a readable web page ({kind})"));
    }
    let (bytes, cut) = read_capped(response, MAX_PAGE_BYTES).await?;
    let body = String::from_utf8_lossy(&bytes);
    let (title, text) = if is_html {
        extract_readable(&body)
    } else {
        (String::new(), tidy_lines(&body))
    };
    let (text, over_budget) = truncate_chars(&text, MAX_PAGE_CHARS);
    Ok(WebPage {
        url: final_url,
        title,
        text,
        truncated: cut || over_budget,
    })
}

/// `Ok` only for `http`/`https` URLs whose host is on the public internet.
pub(super) fn check_public_url(url: &reqwest::Url) -> Result<(), String> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("only http and https pages can be read".to_owned());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "that web address has no host".to_owned())?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return if is_public_ip(ip) {
            Ok(())
        } else {
            Err("that address is on this computer or a private network".to_owned())
        };
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let local_suffix = [
        ".localhost",
        ".local",
        ".internal",
        ".lan",
        ".home.arpa",
        ".intranet",
    ]
    .iter()
    .any(|suffix| host.ends_with(suffix));
    // A single-label name ("router", "nas") only resolves on a local network.
    if host == "localhost" || local_suffix || !host.contains('.') {
        return Err("that address is on this computer or a private network".to_owned());
    }
    Ok(())
}

pub(super) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                // fc00::/7 unique local, fe80::/10 link local
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                // 64:ff9b::/96 NAT64 can wrap a private IPv4 address
                || (first == 0x0064 && ip.segments()[1] == 0xff9b)
                // 2001:db8::/32 documentation
                || (first == 0x2001 && ip.segments()[1] == 0x0db8))
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        // 100.64.0.0/10 carrier-grade NAT
        || (a == 100 && (64..128).contains(&b))
        // 198.18.0.0/15 benchmarking
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4 reserved
        || a >= 240
        // 192.0.0.0/24 protocol assignments
        || (a == 192 && b == 0 && c == 0))
}

/// Title and main readable text of an HTML document.
///
/// Prefers `<article>`, then `<main>`, then the body, and drops navigation,
/// scripts and forms. Walks the tree with an explicit stack, so a pathologically
/// nested page cannot exhaust the thread's stack.
pub(super) fn extract_readable(html: &str) -> (String, String) {
    enum Step<N> {
        Visit(N),
        EndBlock,
    }
    let document = Html::parse_document(html);
    let title = document
        .select(&TITLE)
        .next()
        .or_else(|| document.select(&HEADING).next())
        .map(|element| collapse(&element.text().collect::<String>()))
        .unwrap_or_default();
    let root = CONTENT_ROOTS
        .iter()
        .find_map(|selector| {
            document
                .select(selector)
                .find(|element| element.text().any(|text| !text.trim().is_empty()))
        })
        .unwrap_or_else(|| document.root_element());

    let mut out = String::new();
    let mut stack: Vec<Step<_>> = root.children().rev().map(Step::Visit).collect();
    while let Some(step) = stack.pop() {
        let node = match step {
            Step::EndBlock => {
                out.push('\n');
                continue;
            }
            Step::Visit(node) => node,
        };
        match node.value() {
            Node::Text(text) => out.push_str(text),
            Node::Element(element) => {
                let name = element.name();
                if SKIPPED.contains(&name)
                    || element.attr("hidden").is_some()
                    || element.attr("aria-hidden") == Some("true")
                {
                    continue;
                }
                let block = BLOCKS.contains(&name);
                if block {
                    out.push('\n');
                    stack.push(Step::EndBlock);
                }
                if ElementRef::wrap(node).is_some() {
                    stack.extend(node.children().rev().map(Step::Visit));
                }
            }
            _ => {}
        }
    }
    (title, tidy_lines(&out))
}

/// One block per line, spaces collapsed, blank lines dropped: compact for the model.
fn tidy_lines(text: &str) -> String {
    text.lines()
        .map(collapse)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((cut, _)) => (text[..cut].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> reqwest::Url {
        reqwest::Url::parse(raw).expect("fixture url")
    }

    #[test]
    fn local_and_private_addresses_are_refused() {
        for blocked in [
            "http://localhost:8080/admin",
            "http://127.0.0.1/",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://router/",
            "http://printer.local/",
            "http://nas.home.arpa/",
            "file:///etc/passwd",
            "ftp://example.com/",
        ] {
            assert!(
                check_public_url(&url(blocked)).is_err(),
                "{blocked} was allowed"
            );
        }
        for allowed in [
            "https://en.wikipedia.org/wiki/Rust",
            "http://93.184.215.14/",
            "https://[2606:4700::1111]/",
        ] {
            assert!(
                check_public_url(&url(allowed)).is_ok(),
                "{allowed} was refused"
            );
        }
    }

    #[test]
    fn readable_text_keeps_the_article_and_drops_page_chrome() {
        let html = r#"<!doctype html><html><head><title> Opening hours  </title>
            <style>.x{color:red}</style><script>alert("hi")</script></head>
            <body><nav>Home | About</nav><header>Site banner</header>
            <article><h1>Library hours</h1><p>Open   <b>9am</b> to 5pm.</p>
            <ul><li>Monday</li><li>Tuesday</li></ul>
            <div hidden>secret</div><p aria-hidden="true">decor</p></article>
            <footer>Copyright</footer></body></html>"#;
        let (title, text) = extract_readable(html);
        assert_eq!(title, "Opening hours");
        assert!(text.contains("Library hours"), "{text}");
        assert!(text.contains("Open 9am to 5pm."), "{text}");
        assert!(text.contains("Monday\nTuesday"), "{text}");
        for chrome in [
            "Home | About",
            "Site banner",
            "Copyright",
            "alert",
            "color:red",
            "secret",
            "decor",
        ] {
            assert!(!text.contains(chrome), "{chrome} leaked into {text}");
        }
    }

    #[test]
    fn falls_back_to_body_and_heading_title() {
        let (title, text) =
            extract_readable("<html><body><h1>Only heading</h1><p>Body text</p></body></html>");
        assert_eq!(title, "Only heading");
        assert!(text.contains("Body text"));
    }

    #[test]
    fn deeply_nested_markup_does_not_overflow() {
        let depth = 20_000;
        let html = format!(
            "{}deep text{}",
            "<div>".repeat(depth),
            "</div>".repeat(depth)
        );
        let (_, text) = extract_readable(&html);
        assert!(text.contains("deep text"));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let (text, cut) = truncate_chars("héllo wörld", 4);
        assert_eq!(text, "héll");
        assert!(cut);
        let (text, cut) = truncate_chars("short", 10);
        assert_eq!(text, "short");
        assert!(!cut);
    }
}
