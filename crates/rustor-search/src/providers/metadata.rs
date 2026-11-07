use super::magnet;
use crate::{Candidate, InfoHash, MAX_MAGNET_BYTES};
use serde_json::Value;
pub(super) fn text(value: &Value, key: &str, max: usize) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| s.len() <= max)
        .unwrap_or("")
        .to_owned()
}
pub(super) fn integer(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}
pub(super) fn count(value: &Value, key: &str) -> Option<u32> {
    integer(value, key).and_then(|n| n.try_into().ok())
}
pub(super) fn timestamp(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(value))
        .ok()
        .and_then(|date| date.timestamp_millis().try_into().ok())
}
pub(super) fn candidate(hash: String, name: String) -> Candidate {
    let hash = InfoHash::parse(&hash)
        .map(|h| h.as_str().to_owned())
        .unwrap_or(hash);
    let magnet = magnet::build(&hash, &name);
    Candidate {
        info_hash: hash,
        name,
        magnet,
        size_bytes: None,
        seeders: None,
        leechers: None,
        file_count: None,
        added_unix_ms: None,
        artwork_url: None,
    }
}
// Native BitTorrent supports HTTP(S)/UDP trackers, not WebTorrent's WS(S).
// Remove only those known-incompatible tracker parameters; preserve the rest byte-for-byte.
pub(super) fn native_magnet(value: String) -> String {
    if url::Url::parse(&value)
        .ok()
        .is_some_and(|url| url.fragment().is_some())
    {
        return value;
    }
    let Some(query) = value.strip_prefix("magnet:?") else {
        return value;
    };
    let parts: Vec<_> = query
        .split('&')
        .filter(|part| {
            !url::form_urlencoded::parse(part.as_bytes()).any(|(key, tracker)| {
                key == "tr"
                    && url::Url::parse(&tracker)
                        .ok()
                        .is_some_and(|url| matches!(url.scheme(), "ws" | "wss"))
            })
        })
        .collect();
    format!("magnet:?{}", parts.join("&"))
}
pub(super) fn magnet_hash(value: &str) -> Option<String> {
    if value.len() > MAX_MAGNET_BYTES {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "magnet" {
        return None;
    }
    url.query_pairs()
        .filter(|(key, _)| key == "xt")
        .find_map(|(_, value)| {
            value
                .strip_prefix("urn:btih:")
                .and_then(|hash| InfoHash::parse(hash).ok())
                .map(|h| h.as_str().to_owned())
        })
}
