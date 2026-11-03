use super::http::{Http, MAX_BODY_BYTES, Media};
#[cfg(test)]
use super::{
    http::{client, retry_after},
    magnet::TRACKERS,
};
use crate::{
    Candidate, Category, InfoHash, Provider, ProviderDescriptor, ProviderFailure, ProviderFuture,
    SearchError, SearchQuery,
};
use std::time::Duration;
const ENDPOINT: &str = "https://nyaa.si/";
const NAMESPACE: &str = "https://nyaa.si/xmlns/nyaa";
const MAX_NODES: u32 = 20_000;
const MAX_ROWS: usize = 101;
/// Fixed-origin metadata search. Does not fetch item links or start transfers.
pub struct NyaaProvider {
    descriptor: ProviderDescriptor,
    http: Http,
    endpoint: url::Url,
}
impl NyaaProvider {
    pub fn new() -> Result<Self, SearchError> {
        Ok(Self {
            descriptor: descriptor()?,
            http: Http::new()?,
            endpoint: url::Url::parse(ENDPOINT)
                .map_err(|_| SearchError::InvalidConfig("invalid fixed provider endpoint"))?,
        })
    }
    async fn fetch(&self, query: &SearchQuery) -> Result<Vec<Candidate>, ProviderFailure> {
        let mut url = self.endpoint.clone();
        url.query_pairs_mut()
            .clear()
            .append_pair("page", "rss")
            .append_pair("q", query.as_str())
            .append_pair("c", "0_0")
            .append_pair("f", "0");
        let body = self
            .http
            .get(url, Media::Xml, Duration::from_secs(12))
            .await?;
        tokio::task::spawn_blocking(move || parse_feed(&body))
            .await
            .map_err(|_| ProviderFailure::InvalidResponse)?
    }
}
impl Provider for NyaaProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(self.fetch(query))
    }
}
fn descriptor() -> Result<ProviderDescriptor, SearchError> {
    ProviderDescriptor::new("nyaa", "Nyaa", vec![Category::Anime], true)
}
fn parse_feed(bytes: &[u8]) -> Result<Vec<Candidate>, ProviderFailure> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(ProviderFailure::InvalidResponse);
    }
    let xml = std::str::from_utf8(bytes).map_err(|_| ProviderFailure::InvalidResponse)?;
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    )
    .map_err(|_| ProviderFailure::InvalidResponse)?;
    let rss = doc.root_element();
    if !rss.has_tag_name("rss") || rss.attribute("version") != Some("2.0") {
        return Err(ProviderFailure::InvalidResponse);
    }
    let channel = rss
        .children()
        .find(|n| n.has_tag_name("channel"))
        .ok_or(ProviderFailure::InvalidResponse)?;
    Ok(channel
        .children()
        .filter(|n| n.has_tag_name("item"))
        .take(MAX_ROWS)
        .map(|item| {
            let raw_hash = field(item, Some(NAMESPACE), "infoHash");
            let info_hash = InfoHash::parse(&raw_hash)
                .map(|hash| hash.as_str().to_owned())
                .unwrap_or(raw_hash);
            let name = field(item, None, "title");
            let magnet = super::magnet::build(&info_hash, &name);
            Candidate {
                info_hash,
                name,
                magnet,
                size_bytes: parse_size(&field(item, Some(NAMESPACE), "size")),
                seeders: count(&field(item, Some(NAMESPACE), "seeders")),
                leechers: count(&field(item, Some(NAMESPACE), "leechers")),
                file_count: None,
                added_unix_ms: chrono::DateTime::parse_from_rfc2822(&field(item, None, "pubDate"))
                    .ok()
                    .and_then(|date| u64::try_from(date.timestamp_millis()).ok()),
            }
        })
        .collect())
}
fn field(node: roxmltree::Node<'_, '_>, namespace: Option<&str>, name: &str) -> String {
    let Some(element) = node.children().find(|n| {
        n.is_element() && n.tag_name().namespace() == namespace && n.tag_name().name() == name
    }) else {
        return String::new();
    };
    let mut value = String::new();
    for text in element
        .children()
        .filter_map(|n| if n.is_text() { n.text() } else { None })
    {
        if text.len() > 2048 - value.len() {
            return String::new();
        }
        value.push_str(text);
    }
    value.trim().to_owned()
}
fn count(value: &str) -> Option<u32> {
    value.trim().parse().ok()
}
fn parse_size(value: &str) -> Option<u64> {
    if value.len() > 64 {
        return None;
    }
    let boundary = value.find(|c: char| c.is_ascii_alphabetic())?;
    let number = value[..boundary].trim();
    let unit = value[boundary..].trim().to_ascii_uppercase();
    let multiplier: u128 = match unit.as_str() {
        "B" => 1,
        "KB" => 1000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "TB" => 1_000_000_000_000,
        "KIB" => 1024,
        "MIB" => 1024_u128.pow(2),
        "GIB" => 1024_u128.pow(3),
        "TIB" => 1024_u128.pow(4),
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let scale = 10_u128.pow(fraction.len() as u32);
    let numerator = whole
        .parse::<u128>()
        .ok()?
        .checked_mul(scale)?
        .checked_add(if fraction.is_empty() {
            0
        } else {
            fraction.parse::<u128>().ok()?
        })?;
    u64::try_from(numerator.checked_mul(multiplier)?.checked_add(scale / 2)? / scale).ok()
}
#[cfg(test)]
#[path = "nyaa_tests.rs"]
mod tests;
