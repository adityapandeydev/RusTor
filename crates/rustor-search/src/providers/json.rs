use super::{
    http::{Http, MAX_BODY_BYTES, Media},
    magnet,
};
use crate::{
    Candidate, Category, InfoHash, MAX_MAGNET_BYTES, Provider, ProviderDescriptor, ProviderFailure,
    ProviderFuture, SearchError, SearchQuery,
};
use serde_json::Value;
use std::time::Duration;

const MAX_ROWS: usize = 101;
#[derive(Clone, Copy)]
enum Kind {
    Yts,
    BitTorrented,
    SubsPlease,
}
/// Fixed-endpoint JSON adapters ported from Torlink. Responses contain metadata only.
pub struct JsonProvider {
    kind: Kind,
    descriptor: ProviderDescriptor,
    http: Http,
    endpoints: Vec<url::Url>,
}
impl JsonProvider {
    pub fn yts() -> Result<Self, SearchError> {
        Self::new(Kind::Yts)
    }
    pub fn bittorrented() -> Result<Self, SearchError> {
        Self::new(Kind::BitTorrented)
    }
    pub fn subsplease() -> Result<Self, SearchError> {
        Self::new(Kind::SubsPlease)
    }
    fn new(kind: Kind) -> Result<Self, SearchError> {
        let (id, label, categories, health, urls) = match kind {
            Kind::Yts => (
                "yts",
                "YTS",
                vec![Category::Movies],
                true,
                vec![
                    "https://yts.mx/api/v2/list_movies.json",
                    "https://yts.am/api/v2/list_movies.json",
                    "https://yts.rs/api/v2/list_movies.json",
                ],
            ),
            Kind::BitTorrented => (
                "bittorrented",
                "BitTorrented",
                vec![Category::Movies, Category::Tv],
                true,
                vec!["https://bittorrented.com/api/search/torrents"],
            ),
            Kind::SubsPlease => (
                "subsplease",
                "SubsPlease",
                vec![Category::Anime],
                false,
                vec!["https://subsplease.org/api/"],
            ),
        };
        Ok(Self {
            kind,
            descriptor: ProviderDescriptor::new(id, label, categories, health)?,
            http: Http::new()?,
            endpoints: urls
                .into_iter()
                .map(|url| {
                    url::Url::parse(url)
                        .map_err(|_| SearchError::InvalidConfig("invalid fixed provider endpoint"))
                })
                .collect::<Result<_, _>>()?,
        })
    }
    fn request_url(&self, endpoint: &url::Url, query: &SearchQuery) -> url::Url {
        let mut url = endpoint.clone();
        let mut params = url.query_pairs_mut();
        params.clear();
        match self.kind {
            Kind::Yts => {
                params.append_pair("limit", "50");
                if query.as_str().is_empty() {
                    params.append_pair("sort_by", "date_added");
                } else {
                    params.append_pair("query_term", query.as_str());
                }
            }
            Kind::BitTorrented => {
                params
                    .append_pair("q", query.as_str())
                    .append_pair("type", "video")
                    .append_pair("limit", "50")
                    .append_pair("sortBy", "seeders")
                    .append_pair("sortOrder", "desc");
            }
            Kind::SubsPlease => {
                params.append_pair("tz", "UTC");
                if query.as_str().is_empty() {
                    params.append_pair("f", "latest");
                } else {
                    params
                        .append_pair("f", "search")
                        .append_pair("s", query.as_str());
                }
            }
        }
        drop(params);
        url
    }
    async fn fetch(&self, query: &SearchQuery) -> Result<Vec<Candidate>, ProviderFailure> {
        if matches!(self.kind, Kind::BitTorrented) && query.as_str().chars().count() < 3 {
            return Ok(vec![]);
        }
        let mut failure = ProviderFailure::Unavailable;
        for endpoint in &self.endpoints {
            let kind = self.kind;
            let media = if matches!(kind, Kind::SubsPlease) {
                Media::SubsPleaseJson
            } else {
                Media::Json
            };
            // Three YTS mirrors must fit inside the service's 15-second provider deadline.
            let timeout = Duration::from_secs(if matches!(kind, Kind::Yts) { 4 } else { 12 });
            match self
                .http
                .get(self.request_url(endpoint, query), media, timeout)
                .await
            {
                Ok(body) => {
                    let decoded = tokio::task::spawn_blocking(move || decode(kind, &body))
                        .await
                        .map_err(|_| ProviderFailure::InvalidResponse)?;
                    match decoded {
                        Ok(rows) => return Ok(rows),
                        Err(error) => failure = error,
                    }
                }
                Err(error @ ProviderFailure::RateLimited { .. }) => return Err(error),
                Err(error) => failure = error,
            }
        }
        Err(failure)
    }
}
impl Provider for JsonProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(self.fetch(query))
    }
}
fn text(value: &Value, key: &str, max: usize) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| s.len() <= max)
        .unwrap_or("")
        .to_owned()
}
fn integer(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}
fn count(value: &Value, key: &str) -> Option<u32> {
    integer(value, key).and_then(|n| n.try_into().ok())
}
fn timestamp(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(value))
        .ok()
        .and_then(|date| date.timestamp_millis().try_into().ok())
}
fn candidate(hash: String, name: String) -> Candidate {
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
    }
}
fn decode(kind: Kind, body: &[u8]) -> Result<Vec<Candidate>, ProviderFailure> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ProviderFailure::InvalidResponse);
    }
    let root: Value = serde_json::from_slice(body).map_err(|_| ProviderFailure::InvalidResponse)?;
    let invalid = || ProviderFailure::InvalidResponse;
    let mut out = Vec::new();
    match kind {
        Kind::Yts => {
            if root
                .get("status")
                .is_some_and(|status| status.as_str() != Some("ok"))
            {
                return Err(invalid());
            }
            let data = root
                .get("data")
                .filter(|v| v.is_object())
                .ok_or_else(invalid)?;
            let movies = match data.get("movies") {
                Some(Value::Array(rows)) => rows,
                None | Some(Value::Null)
                    if data.get("movie_count").and_then(Value::as_u64) == Some(0) =>
                {
                    return Ok(out);
                }
                _ => return Err(invalid()),
            };
            for movie in movies.iter().take(MAX_ROWS) {
                let title = text(movie, "title_long", 1024);
                let title = if title.is_empty() {
                    text(movie, "title", 1024)
                } else {
                    title
                };
                let Some(torrents) = movie.get("torrents").and_then(Value::as_array) else {
                    continue;
                };
                for torrent in torrents.iter().take(MAX_ROWS - out.len()) {
                    let quality = text(torrent, "quality", 32);
                    let format = text(torrent, "type", 32);
                    let tag = [quality, format]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let name = if title.is_empty() || tag.is_empty() {
                        title.clone()
                    } else {
                        format!("{title} [{tag}]")
                    };
                    let mut row = candidate(text(torrent, "hash", 64), name);
                    row.size_bytes = integer(torrent, "size_bytes");
                    row.seeders = count(torrent, "seeds");
                    row.leechers = count(torrent, "peers");
                    row.added_unix_ms = integer(movie, "date_uploaded_unix")
                        .and_then(|n| n.checked_mul(1000))
                        .or_else(|| timestamp(&text(movie, "date_uploaded", 128)));
                    out.push(row);
                }
                if out.len() == MAX_ROWS {
                    break;
                }
            }
        }
        Kind::BitTorrented => {
            let rows = root
                .get("results")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            for value in rows.iter().take(MAX_ROWS) {
                let mut row = candidate(
                    text(value, "torrent_infohash", 64),
                    text(value, "torrent_name", 1024),
                );
                row.size_bytes = integer(value, "torrent_total_size");
                row.seeders = count(value, "torrent_seeders");
                row.leechers = count(value, "torrent_leechers");
                row.file_count = count(value, "torrent_file_count");
                row.added_unix_ms = timestamp(&text(value, "torrent_created_at", 128));
                out.push(row);
            }
        }
        Kind::SubsPlease => {
            if root.as_array().is_some_and(Vec::is_empty) {
                return Ok(out);
            }
            let entries = root.as_object().ok_or_else(invalid)?;
            for value in entries.values().take(MAX_ROWS) {
                let downloads = value.get("downloads").and_then(Value::as_array);
                let valid = |v: &&Value| {
                    let m = text(v, "magnet", MAX_MAGNET_BYTES);
                    magnet_hash(&m).is_some()
                };
                let preferred = downloads.and_then(|downloads| {
                    ["1080", "720", "480"]
                        .iter()
                        .find_map(|res| {
                            downloads
                                .iter()
                                .filter(valid)
                                .find(|dl| dl.get("res").and_then(Value::as_str) == Some(res))
                        })
                        .or_else(|| downloads.iter().find(valid))
                });
                let magnet = preferred
                    .map(|dl| text(dl, "magnet", MAX_MAGNET_BYTES))
                    .unwrap_or_default();
                let hash = magnet_hash(&magnet).unwrap_or_default();
                let show = text(value, "show", 1024);
                let episode = text(value, "episode", 64);
                let res = preferred.map(|dl| text(dl, "res", 16)).unwrap_or_default();
                let name = if show.is_empty() {
                    show
                } else {
                    format!(
                        "{show}{} [{}p]",
                        if episode.is_empty() {
                            String::new()
                        } else {
                            format!(" - {episode}")
                        },
                        if res.is_empty() { "?" } else { &res }
                    )
                };
                let mut row = candidate(hash, name);
                row.magnet = native_magnet(magnet);
                row.size_bytes = url::Url::parse(&row.magnet).ok().and_then(|url| {
                    url.query_pairs()
                        .find(|(key, _)| key == "xl")
                        .and_then(|(_, value)| value.parse().ok())
                });
                row.added_unix_ms = timestamp(&text(value, "release_date", 128));
                out.push(row);
            }
        }
    }
    Ok(out)
}
// Native BitTorrent supports HTTP(S)/UDP trackers, not WebTorrent's WS(S).
// Remove only those known-incompatible tracker parameters; preserve the rest byte-for-byte.
fn native_magnet(value: String) -> String {
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
fn magnet_hash(value: &str) -> Option<String> {
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
#[cfg(test)]
#[path = "json_tests.rs"]
mod tests;
