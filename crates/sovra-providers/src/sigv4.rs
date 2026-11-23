//! AWS Signature Version 4 for the S3 language, written out with the hashing
//! Sovra already carries so no storage SDK is built in.
//!
//! Shared by the training uploader and online storage. A key is never written
//! to a log or an error.

use sha2::{Digest, Sha256};

/// Most of a service's answer that is read when something goes wrong.
const MAX_ERROR_BYTES: usize = 4096;

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    out
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// HMAC-SHA256 (RFC 2104), written out because it is a dozen lines and the
/// hashing is already here.
pub(crate) fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0_u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(message);
    let inner: [u8; 32] = inner.finalize().into();
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

/// AWS's own percent-encoding: everything but letters, digits and `-._~`, with
/// `/` kept when it separates the parts of a file's name.
pub(crate) fn uri_encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && byte == b'/')
        {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push_str(&hex(&[byte]).to_ascii_uppercase());
        }
    }
    out
}

/// A moment as AWS writes it: `YYYYMMDD` and `YYYYMMDDTHHMMSSZ`, in UTC.
pub(crate) fn stamps(unix_seconds: u64) -> (String, String) {
    let days = unix_seconds / 86_400;
    let second_of_day = unix_seconds % 86_400;
    // Days since 1970 to a calendar date (Howard Hinnant's civil-from-days).
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    let date = format!("{year:04}{month:02}{day:02}");
    let time = format!(
        "{date}T{:02}{:02}{:02}Z",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60
    );
    (date, time)
}

/// One request, ready to sign.
pub(crate) struct Signing<'a> {
    pub(crate) method: &'a str,
    /// The path as it is sent, already percent-encoded.
    pub(crate) path: &'a str,
    /// The query string as it is sent, made by [`canonical_query`].
    pub(crate) query: &'a str,
    /// Header names in lower case with their values, sorted by name.
    pub(crate) headers: &'a [(&'a str, String)],
    pub(crate) payload_hash: &'a str,
    pub(crate) region: &'a str,
    pub(crate) access_key_id: &'a str,
    pub(crate) secret: &'a str,
    pub(crate) date: &'a str,
    pub(crate) time: &'a str,
}

/// The `Authorization` header for a request.
pub(crate) fn authorization(request: &Signing<'_>) -> String {
    let mut canonical_headers = String::new();
    for (name, value) in request.headers {
        canonical_headers.push_str(name);
        canonical_headers.push(':');
        canonical_headers.push_str(value.trim());
        canonical_headers.push('\n');
    }
    let signed: Vec<&str> = request.headers.iter().map(|(name, _)| *name).collect();
    let signed = signed.join(";");
    let canonical = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed}\n{}",
        request.method, request.path, request.query, request.payload_hash
    );
    let scope = format!("{}/{}/s3/aws4_request", request.date, request.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        request.time,
        hex(&sha256(canonical.as_bytes()))
    );
    let mut key = hmac(
        format!("AWS4{}", request.secret).as_bytes(),
        request.date.as_bytes(),
    );
    for part in [request.region, "s3", "aws4_request"] {
        key = hmac(&key, part.as_bytes());
    }
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
        request.access_key_id
    )
}

/// The service's own short error name (`AccessDenied`, `NoSuchBucket`) out of
/// its answer, if it gave one made only of letters and digits.
pub(crate) fn error_code(body: &str) -> Option<&str> {
    let start = body.find("<Code>")? + 6;
    let end = body[start..].find("</Code>")? + start;
    let code = &body[start..end];
    (!code.is_empty() && code.len() <= 64 && code.chars().all(|c| c.is_ascii_alphanumeric()))
        .then_some(code)
}

/// A query string in the order and encoding AWS signs it: names sorted, every
/// name and value percent-encoded. The same text is sent.
pub(crate) fn canonical_query(pairs: &[(&str, &str)]) -> String {
    let mut encoded: Vec<(String, String)> = pairs
        .iter()
        .map(|(name, value)| (uri_encode(name, false), uri_encode(value, false)))
        .collect();
    encoded.sort();
    encoded
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// The first bytes of an answer as text, for reading an error out of.
pub(crate) async fn error_text(mut response: reqwest::Response) -> String {
    let mut bytes = Vec::new();
    while bytes.len() < MAX_ERROR_BYTES {
        match response.chunk().await {
            Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
            Ok(None) | Err(_) => break,
        }
    }
    bytes.truncate(MAX_ERROR_BYTES);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_matches_the_worked_example_in_the_aws_documentation() {
        // "Signature Calculations for the Authorization Header", PUT Object.
        let body = b"Welcome to Amazon S3.";
        let payload_hash = hex(&sha256(body));
        assert_eq!(
            payload_hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let headers = [
            ("date", "Fri, 24 May 2013 00:00:00 GMT".to_owned()),
            ("host", "examplebucket.s3.amazonaws.com".to_owned()),
            ("x-amz-content-sha256", payload_hash.clone()),
            ("x-amz-date", "20130524T000000Z".to_owned()),
            ("x-amz-storage-class", "REDUCED_REDUNDANCY".to_owned()),
        ];
        let path = format!("/{}", uri_encode("test$file.text", true));
        assert_eq!(path, "/test%24file.text");
        let header = authorization(&Signing {
            method: "PUT",
            path: &path,
            query: "",
            headers: &headers,
            payload_hash: &payload_hash,
            region: "us-east-1",
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            date: "20130524",
            time: "20130524T000000Z",
        });
        assert_eq!(
            header,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class, Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn hmac_matches_the_published_test_vectors() {
        // RFC 4231, test cases 1, 2 and 6 (a key longer than one block).
        assert_eq!(
            hex(&hmac(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hex(&hmac(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn listing_with_a_query_matches_the_aws_documentation() {
        // "Signature Calculations for the Authorization Header", GET Bucket.
        let payload_hash = hex(&sha256(b""));
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com".to_owned()),
            ("x-amz-content-sha256", payload_hash.clone()),
            ("x-amz-date", "20130524T000000Z".to_owned()),
        ];
        let query = canonical_query(&[("prefix", "J"), ("max-keys", "2")]);
        assert_eq!(query, "max-keys=2&prefix=J");
        let header = authorization(&Signing {
            method: "GET",
            path: "/",
            query: &query,
            headers: &headers,
            payload_hash: &payload_hash,
            region: "us-east-1",
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            date: "20130524",
            time: "20130524T000000Z",
        });
        assert!(
            header.ends_with(
                "SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
            ),
            "{header}"
        );
        // An empty value and a slash are still encoded.
        assert_eq!(
            canonical_query(&[
                ("list-type", "2"),
                ("delimiter", "/"),
                ("continuation-token", "")
            ]),
            "continuation-token=&delimiter=%2F&list-type=2"
        );
    }
}
